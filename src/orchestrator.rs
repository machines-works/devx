use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, DebouncedEventKind};
use tokio::sync::mpsc;

use crate::config::DevxConfig;
use crate::deps;
use crate::events::DevxEvent;
use crate::git;
use crate::infra;
use crate::ports::{PortAllocator, PortAllocation};
use crate::process::ManagedProcess;
use crate::proxy::{ServiceProxy, VhostProxy};
use crate::tls;
use crate::control;
use crate::watcher;

pub enum OrchestratorCommand {
    Restart { service: String },
    ReloadConfig,
    Shutdown,
    Status {
        reply: tokio::sync::oneshot::Sender<String>,
    },
}

pub struct Orchestrator {
    config: DevxConfig,
    project_root: PathBuf,
    processes: HashMap<String, ManagedProcess>,
    actual_ports: HashMap<String, u16>,
    proxy_ports: HashMap<String, u16>,
    /// Shared atomic targets so proxy tasks pick up new ports after restart
    proxy_targets: HashMap<String, Arc<AtomicU16>>,
    allocator: PortAllocator,
    event_tx: mpsc::Sender<DevxEvent>,
    cmd_tx: mpsc::Sender<OrchestratorCommand>,
    cmd_rx: Option<mpsc::Receiver<OrchestratorCommand>>,
}

impl Orchestrator {
    pub fn new(
        config: DevxConfig,
        project_root: PathBuf,
        event_tx: mpsc::Sender<DevxEvent>,
    ) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        Self {
            config,
            project_root,
            processes: HashMap::new(),
            actual_ports: HashMap::new(),
            proxy_ports: HashMap::new(),
            proxy_targets: HashMap::new(),
            allocator: PortAllocator::new(),
            event_tx,
            cmd_tx,
            cmd_rx: Some(cmd_rx),
        }
    }

    pub fn cmd_sender(&self) -> mpsc::Sender<OrchestratorCommand> {
        self.cmd_tx.clone()
    }

    fn dep_graph(&self) -> HashMap<String, Vec<String>> {
        self.config
            .services
            .iter()
            .map(|(name, svc)| (name.clone(), svc.depends_on.clone()))
            .collect()
    }

    pub async fn start(&mut self, filter: Option<&[String]>) -> Result<()> {
        if let Some(infra_cfg) = &self.config.infra {
            let compose_path = self.project_root.join(&infra_cfg.compose);
            infra::ensure_compose(&compose_path)?;
        }

        let graph = self.dep_graph();

        let waves = match filter {
            Some(requested) => deps::resolve_with_filter(&graph, requested)?,
            None => deps::resolve_order(&graph)?,
        };

        for wave in &waves {
            for name in wave {
                let svc = self
                    .config
                    .services
                    .get(name)
                    .ok_or_else(|| anyhow::anyhow!("service '{}' not found in config", name))?;

                if let Some(preferred_port) = svc.port {
                    // Service has a port: allocate_any for the service, preferred for proxy
                    let service_alloc = self.allocator.allocate_any(name)?;
                    self.actual_ports.insert(name.clone(), service_alloc.actual);

                    let proxy_alloc = self.allocator.allocate(name, preferred_port)?;
                    self.proxy_ports.insert(name.clone(), proxy_alloc.actual);
                } else {
                    // No port: allocate_any, no proxy
                    let service_alloc = self.allocator.allocate_any(name)?;
                    self.actual_ports.insert(name.clone(), service_alloc.actual);
                }
            }
        }

        // Generate TLS config if enabled
        let tls_config: Option<Arc<rustls::ServerConfig>> = if self.config.tls_enabled() {
            match self.build_tls_config() {
                Ok(cfg) => Some(cfg),
                Err(e) => {
                    let _ = self.event_tx.try_send(DevxEvent::LogLine {
                        service: "devx".to_string(),
                        line: format!("[tls] failed to generate certs, falling back to HTTP: {}", e),
                        is_stderr: true,
                    });
                    None
                }
            }
        } else {
            None
        };

        // Build shared atomic targets for every service that has a port allocation.
        // Both ServiceProxy and VhostProxy will share these so restarts propagate.
        for (name, &actual_port) in &self.actual_ports {
            self.proxy_targets
                .entry(name.clone())
                .or_insert_with(|| Arc::new(AtomicU16::new(actual_port)));
        }

        let proxy_services: Vec<(String, u16, Arc<AtomicU16>)> = self
            .config
            .services
            .iter()
            .filter(|(_, svc)| svc.port.is_some())
            .filter_map(|(name, _)| {
                let proxy_port = self.proxy_ports.get(name).copied()?;
                let atomic_target = self.proxy_targets.get(name).cloned()?;
                Some((name.clone(), proxy_port, atomic_target))
            })
            .collect();

        for (name, proxy_port, target_port) in proxy_services {
            let proxy = ServiceProxy {
                service_name: name.clone(),
                listen_port: proxy_port,
                target_port,
                tls_config: tls_config.clone(),
            };
            let tx = self.event_tx.clone();
            let tx2 = self.event_tx.clone();
            let name2 = name.clone();
            tokio::spawn(async move {
                if let Err(e) = proxy.run(tx).await {
                    let _ = tx2.try_send(DevxEvent::LogLine {
                        service: "devx".to_string(),
                        line: format!("[proxy] {} error: {}", name2, e),
                        is_stderr: true,
                    });
                }
            });
        }

        // Start vhost proxy if any service has a domain
        let mut vhost_routes: HashMap<String, Arc<AtomicU16>> = self
            .config
            .services
            .iter()
            .filter_map(|(name, svc)| {
                let domain = svc.domain.as_ref()?;
                let atomic_target = self.proxy_targets.get(name).cloned()?;
                Some((domain.clone(), atomic_target))
            })
            .collect();

        // Add branch-prefixed domain variants (e.g. fix-auth.api.localhost -> same port)
        let branch_extras: Vec<(String, Arc<AtomicU16>)> = vhost_routes
            .iter()
            .filter_map(|(domain, port)| {
                let branch_dom = git::branch_domain(domain)?;
                Some((branch_dom, Arc::clone(port)))
            })
            .collect();
        vhost_routes.extend(branch_extras);

        if !vhost_routes.is_empty() {
            let domain_services: Vec<(String, String)> = self
                .config
                .services
                .iter()
                .filter_map(|(name, svc)| {
                    let domain = svc.domain.as_ref()?;
                    Some((domain.clone(), name.clone()))
                })
                .collect();

            let vhost = VhostProxy {
                routes: vhost_routes,
                domain_services,
                tls_config: tls_config.clone(),
            };
            let tx = self.event_tx.clone();
            let tx2 = self.event_tx.clone();
            tokio::spawn(async move {
                if let Err(e) = vhost.run(tx).await {
                    let _ = tx2.try_send(DevxEvent::LogLine {
                        service: "devx".to_string(),
                        line: format!("[vhost] error: {}", e),
                        is_stderr: true,
                    });
                }
            });
        }

        for wave in &waves {
            for name in wave {
                let svc = self.config.services.get(name)
                    .ok_or_else(|| anyhow::anyhow!("service '{}' not found in config", name))?
                    .clone();
                let port_alloc = PortAllocation {
                    preferred: svc.port,
                    actual: *self.actual_ports.get(name)
                        .ok_or_else(|| anyhow::anyhow!("no port allocated for '{}'", name))?,
                    remapped: false,
                };

                let mut proc = ManagedProcess::new(name.clone(), svc, port_alloc);
                proc.spawn(
                    &self.project_root,
                    &self.actual_ports,
                    &self.proxy_ports,
                    self.event_tx.clone(),
                )
                .await?;
                self.processes.insert(name.clone(), proc);
            }
        }

        // Start file watchers for services that have watch enabled
        for (name, svc) in &self.config.services {
            if !svc.watch {
                continue;
            }
            let watch_dir = match &svc.dir {
                Some(dir) => self.project_root.join(dir),
                None => self.project_root.clone(),
            };
            if watch_dir.is_dir() {
                if let Err(e) = watcher::start_watcher(
                    name.clone(),
                    watch_dir,
                    self.event_tx.clone(),
                ) {
                    let _ = self.event_tx.try_send(DevxEvent::LogLine {
                        service: "devx".to_string(),
                        line: format!("[watch] failed to watch {}: {}", name, e),
                        is_stderr: true,
                    });
                }
            }
        }

        // Watch devx.toml for config changes
        let config_path = self.project_root.join("devx.toml");
        if config_path.is_file() {
            let cmd_tx = self.cmd_tx.clone();
            let config_dir = self.project_root.clone();
            let event_tx_err = self.event_tx.clone();
            let watch_result = (|| -> notify::Result<()> {
                let mut debouncer = new_debouncer(
                    Duration::from_millis(500),
                    move |result: Result<Vec<notify_debouncer_mini::DebouncedEvent>, notify::Error>| {
                        let events = match result {
                            Ok(events) => events,
                            Err(_) => return,
                        };
                        let config_changed = events.iter().any(|e| {
                            e.kind == DebouncedEventKind::Any
                                && e.path.file_name().and_then(|f| f.to_str()) == Some("devx.toml")
                        });
                        if config_changed {
                            let _ = cmd_tx.try_send(OrchestratorCommand::ReloadConfig);
                        }
                    },
                )?;
                debouncer.watcher().watch(&config_dir, RecursiveMode::NonRecursive)?;
                std::mem::forget(debouncer);
                Ok(())
            })();
            if let Err(e) = watch_result {
                let _ = event_tx_err.try_send(DevxEvent::LogLine {
                    service: "devx".to_string(),
                    line: format!("[watch] failed to watch devx.toml: {}", e),
                    is_stderr: true,
                });
            }
        }

        // Start control socket server
        let ctrl_project = self.config.project.name.clone();
        let ctrl_tx = self.event_tx.clone();
        let ctrl_tx2 = self.event_tx.clone();
        tokio::spawn(async move {
            if let Err(e) = control::serve(&ctrl_project, ctrl_tx).await {
                let _ = ctrl_tx2.try_send(DevxEvent::LogLine {
                    service: "devx".to_string(),
                    line: format!("[control] socket error: {}", e),
                    is_stderr: true,
                });
            }
        });

        let _ = self.event_tx.try_send(DevxEvent::AllStarted);

        Ok(())
    }

    /// Run the command loop, processing restart and reload commands until shutdown.
    pub async fn run_loop(&mut self) {
        let mut cmd_rx = match self.cmd_rx.take() {
            Some(rx) => rx,
            None => return,
        };

        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                OrchestratorCommand::Restart { service } => {
                    if let Err(e) = self.restart(&service).await {
                        let _ = self.event_tx.try_send(DevxEvent::LogLine {
                            service: "devx".to_string(),
                            line: format!("[restart] failed to restart {}: {}", service, e),
                            is_stderr: true,
                        });
                    }
                }
                OrchestratorCommand::ReloadConfig => {
                    let config_path = self.project_root.join("devx.toml");
                    match DevxConfig::load(&config_path) {
                        Ok(new_config) => {
                            if let Err(e) = self.reload_config(new_config).await {
                                let _ = self.event_tx.try_send(DevxEvent::LogLine {
                                    service: "devx".to_string(),
                                    line: format!("[reload] error: {}", e),
                                    is_stderr: true,
                                });
                            }
                        }
                        Err(e) => {
                            let _ = self.event_tx.try_send(DevxEvent::LogLine {
                                service: "devx".to_string(),
                                line: format!("[reload] failed to parse devx.toml: {}", e),
                                is_stderr: true,
                            });
                        }
                    }
                }
                OrchestratorCommand::Status { reply } => {
                    let statuses: Vec<serde_json::Value> = self
                        .processes
                        .iter()
                        .map(|(name, proc)| {
                            let uptime = proc.started_at.map(|t| t.elapsed().as_secs());
                            serde_json::json!({
                                "name": name,
                                "state": proc.state.label(),
                                "port": self.actual_ports.get(name),
                                "proxy_port": self.proxy_ports.get(name),
                                "uptime_secs": uptime,
                            })
                        })
                        .collect();
                    let response = serde_json::json!({
                        "services": statuses,
                        "project": self.config.project.name,
                    });
                    let _ = reply.send(response.to_string());
                }
                OrchestratorCommand::Shutdown => {
                    let _ = self.shutdown().await;
                    break;
                }
            }
        }
    }

    fn build_tls_config(&self) -> Result<Arc<rustls::ServerConfig>> {
        let (ca_cert_pem, ca_key_pem) = tls::ensure_ca()?;

        let mut domains: Vec<String> = self
            .config
            .services
            .values()
            .filter_map(|svc| svc.domain.clone())
            .collect();

        // Add *.localhost wildcard
        if !domains.iter().any(|d| d == "*.localhost") {
            domains.push("*.localhost".to_string());
        }

        tls::generate_server_config(&ca_cert_pem, &ca_key_pem, &domains)
    }

    pub async fn restart(&mut self, name: &str) -> Result<()> {
        // Stop existing process
        if let Some(proc) = self.processes.get_mut(name) {
            proc.stop().await?;
        }

        let svc = self
            .config
            .services
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("service '{}' not found", name))?
            .clone();

        // Re-allocate port (reuses the session allocator to avoid collisions)
        let service_alloc = self.allocator.allocate_any(name)?;
        self.actual_ports.insert(name.to_string(), service_alloc.actual);

        // Update shared atomic so proxies (ServiceProxy + VhostProxy) forward to the new port
        if let Some(atomic_target) = self.proxy_targets.get(name) {
            atomic_target.store(service_alloc.actual, Ordering::Relaxed);
        }

        let port_alloc = PortAllocation {
            preferred: svc.port,
            actual: service_alloc.actual,
            remapped: false,
        };

        // Respawn
        let mut proc = ManagedProcess::new(name.to_string(), svc, port_alloc);
        proc.spawn(
            &self.project_root,
            &self.actual_ports,
            &self.proxy_ports,
            self.event_tx.clone(),
        )
        .await?;
        self.processes.insert(name.to_string(), proc);

        Ok(())
    }

    pub async fn reload_config(&mut self, new_config: DevxConfig) -> Result<()> {
        let diff = self.config.diff(&new_config);

        // Stop removed services
        for name in &diff.removed {
            if let Some(mut proc) = self.processes.remove(name) {
                let _ = proc.stop().await;
            }
        }

        // Update config before restarting/starting so restart() picks up new values
        self.config = new_config;

        // Restart changed services
        for name in &diff.changed {
            if let Err(e) = self.restart(name).await {
                let _ = self.event_tx.try_send(DevxEvent::LogLine {
                    service: "devx".to_string(),
                    line: format!("[reload] failed to restart {}: {}", name, e),
                    is_stderr: true,
                });
            }
        }

        // Start added services
        for name in &diff.added {
            let svc = match self.config.services.get(name) {
                Some(svc) => svc.clone(),
                None => continue,
            };

            // Allocate ports
            if let Some(preferred_port) = svc.port {
                let service_alloc = self.allocator.allocate_any(name)?;
                self.actual_ports.insert(name.clone(), service_alloc.actual);
                let proxy_alloc = self.allocator.allocate(name, preferred_port)?;
                self.proxy_ports.insert(name.clone(), proxy_alloc.actual);
            } else {
                let service_alloc = self.allocator.allocate_any(name)?;
                self.actual_ports.insert(name.clone(), service_alloc.actual);
            }

            let port_alloc = PortAllocation {
                preferred: svc.port,
                actual: *self.actual_ports.get(name)
                    .ok_or_else(|| anyhow::anyhow!("no port allocated for '{}'", name))?,
                remapped: false,
            };

            let mut proc = ManagedProcess::new(name.clone(), svc, port_alloc);
            proc.spawn(
                &self.project_root,
                &self.actual_ports,
                &self.proxy_ports,
                self.event_tx.clone(),
            )
            .await?;
            self.processes.insert(name.clone(), proc);
        }

        let _ = self.event_tx.try_send(DevxEvent::ConfigReloaded { diff });

        Ok(())
    }

    pub async fn shutdown(&mut self) -> Result<()> {
        for (_, proc) in self.processes.iter_mut() {
            let _ = proc.stop().await;
        }
        Ok(())
    }

    pub fn service_names(&self) -> Vec<String> {
        let graph = self.dep_graph();
        match deps::resolve_order(&graph) {
            Ok(waves) => waves.into_iter().flatten().collect(),
            Err(_) => self.config.services.keys().cloned().collect(),
        }
    }
}
