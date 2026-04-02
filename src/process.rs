use crate::config::{ServiceConfig, interpolate};
use crate::detect::Framework;
use crate::events::{DevxEvent, ServiceState};
use crate::ports::PortAllocation;
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Child;
use tokio::sync::mpsc;

pub struct ManagedProcess {
    pub name: String,
    pub config: ServiceConfig,
    pub port: PortAllocation,
    pub child: Option<Child>,
    pub pid: Option<u32>,
    pub state: ServiceState,
    pub started_at: Option<Instant>,
}

impl ManagedProcess {
    pub fn new(name: String, config: ServiceConfig, port: PortAllocation) -> Self {
        Self {
            name,
            config,
            port,
            child: None,
            pid: None,
            state: ServiceState::Pending,
            started_at: None,
        }
    }

    pub async fn spawn(
        &mut self,
        project_root: &Path,
        actual_ports: &HashMap<String, u16>,
        proxy_ports: &HashMap<String, u16>,
        event_tx: mpsc::Sender<DevxEvent>,
    ) -> Result<()> {
        let cmd = interpolate(&self.config.cmd, &self.name, actual_ports, proxy_ports);

        let work_dir = match &self.config.dir {
            Some(dir) => project_root.join(dir),
            None => project_root.to_path_buf(),
        };

        // Auto-detect framework and inject port if cmd doesn't already use ${port}
        let actual_port = actual_ports.get(&self.name).copied().unwrap_or(0);
        let framework = Framework::detect(&cmd, &work_dir);
        // Skip port injection for self-managed frameworks (e.g., Encore) or
        // when the config explicitly sets managed = false
        let cmd = if framework.self_managed() || !self.config.managed {
            cmd
        } else {
            framework.inject_port_flag(&cmd, actual_port).unwrap_or(cmd)
        };

        let mut command = tokio::process::Command::new("sh");
        command
            .arg("-c")
            .arg(&cmd)
            .current_dir(&work_dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);

        // Create a new session (process group) so we can kill the entire tree later.
        // SAFETY: setsid() is async-signal-safe and is called in the forked child
        // before exec. It creates a new session/process group with the child as leader.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }

        // Load .env file if specified
        if let Some(env_file) = &self.config.env_file {
            let env_path = work_dir.join(env_file);
            if let Ok(iter) = dotenvy::from_path_iter(&env_path) {
                for item in iter.flatten() {
                    let (key, val) = item;
                    // Don't override explicit env entries from devx.toml
                    if !self.config.env.contains_key(&key) {
                        let interpolated = interpolate(&val, &self.name, actual_ports, proxy_ports);
                        command.env(key, interpolated);
                    }
                }
            }
        }

        for (key, val) in &self.config.env {
            let interpolated_val = interpolate(val, &self.name, actual_ports, proxy_ports);
            command.env(key, interpolated_val);
        }

        // Inject PORT env var for frameworks that use it (unless already set in config
        // or the service is self-managed)
        if framework.injects_port_env()
            && self.config.managed
            && !framework.self_managed()
            && !self.config.env.contains_key("PORT")
        {
            command.env("PORT", actual_port.to_string());
        }

        let mut child = command.spawn()?;

        // Capture the child PID before we hand off the Child handle.
        // Since we called setsid(), the child PID is also the process group ID (PGID).
        let child_pid = child
            .id()
            .expect("child should have a PID right after spawn");

        if let Some(stdout) = child.stdout.take() {
            let service = self.name.clone();
            let tx = event_tx.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    let _ = tx.try_send(DevxEvent::LogLine {
                        service: service.clone(),
                        line,
                        is_stderr: false,
                    });
                }
            });
        }

        if let Some(stderr) = child.stderr.take() {
            let service = self.name.clone();
            let tx = event_tx.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    let _ = tx.try_send(DevxEvent::LogLine {
                        service: service.clone(),
                        line,
                        is_stderr: true,
                    });
                }
            });
        }

        self.child = Some(child);
        self.pid = Some(child_pid);
        self.state = ServiceState::Starting;
        self.started_at = Some(Instant::now());

        let _ = event_tx.try_send(DevxEvent::StateChange {
            service: self.name.clone(),
            state: ServiceState::Starting,
            started_at: self.started_at,
        });

        let service = self.name.clone();
        let health_url = self
            .config
            .health
            .as_ref()
            .map(|h| interpolate(h, &self.name, actual_ports, proxy_ports));
        let tx = event_tx.clone();
        let started_at = self.started_at;

        tokio::spawn(async move {
            if let Some(url) = health_url {
                // Poll every 2s, up to 30 retries (60s total)
                let client = reqwest::Client::new();
                let max_retries = 30;
                let mut healthy = false;
                for _ in 0..max_retries {
                    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

                    // Short-circuit: check if the process has already exited.
                    // kill(pid, 0) checks existence without sending a signal.
                    // SAFETY: just a process-existence probe, no side effects.
                    let alive = unsafe { libc::kill(child_pid as libc::pid_t, 0) } == 0;
                    if !alive {
                        let _ = tx.try_send(DevxEvent::StateChange {
                            service: service.clone(),
                            state: ServiceState::Failed("process exited".into()),
                            started_at,
                        });
                        return;
                    }

                    match client.get(&url).send().await {
                        Ok(resp) if resp.status().is_success() => {
                            let _ = tx.try_send(DevxEvent::StateChange {
                                service: service.clone(),
                                state: ServiceState::Healthy,
                                started_at,
                            });
                            healthy = true;
                            break;
                        }
                        _ => continue,
                    }
                }
                if !healthy {
                    let _ = tx.try_send(DevxEvent::StateChange {
                        service: service.clone(),
                        state: ServiceState::Unhealthy,
                        started_at,
                    });
                }
            } else {
                // No health check — wait 3s then mark Healthy
                tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
                let _ = tx.try_send(DevxEvent::StateChange {
                    service: service.clone(),
                    state: ServiceState::Healthy,
                    started_at,
                });
            }
        });

        Ok(())
    }

    pub async fn stop(&mut self) -> Result<()> {
        if let Some(mut child) = self.child.take() {
            // Use stored PID (which is also the PGID since we called setsid in spawn).
            let pgid = self.pid;

            // Send SIGTERM to the entire process group.
            if let Some(pid) = pgid {
                // SAFETY: killpg sends a signal to the process group.
                unsafe {
                    libc::killpg(pid as libc::pid_t, libc::SIGTERM);
                }
            }

            // Wait up to 5 seconds for the child to exit gracefully.
            let graceful =
                tokio::time::timeout(tokio::time::Duration::from_secs(5), child.wait()).await;

            if graceful.is_err() {
                // Timeout expired — escalate to SIGKILL on the process group.
                if let Some(pid) = pgid {
                    unsafe {
                        libc::killpg(pid as libc::pid_t, libc::SIGKILL);
                    }
                }
                // Reap the zombie (with a timeout so we never hang).
                let _ =
                    tokio::time::timeout(tokio::time::Duration::from_secs(2), child.wait()).await;
            }
        }
        self.pid = None;
        self.state = ServiceState::Stopped;
        Ok(())
    }
}
