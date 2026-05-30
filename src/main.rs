use std::collections::VecDeque;
use std::io::{BufRead, Seek, Write};
use std::path::PathBuf;
use std::time::SystemTime;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use tokio::sync::mpsc;

use devx::config::DevxConfig;
use devx::control;
use devx::daemon;
use devx::events::DevxEvent;
use devx::infra;
use devx::init;
use devx::orchestrator::Orchestrator;
use devx::project::resolve_project_id;
use devx::tls;
use devx::tui::App;

#[derive(Parser)]
#[command(name = "devx", about = "Local development orchestrator")]
struct Cli {
    /// Override the project instance id (socket/PID/log key). Defaults to
    /// [project].name, or "{name}-{worktree-dir}" inside a linked git worktree.
    #[arg(short = 'p', long = "project", global = true)]
    project: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start all services
    Up {
        /// Run in background (no TUI)
        #[arg(short = 'd', long = "daemon")]
        daemon: bool,
        /// Specific services to start (starts all if empty)
        services: Vec<String>,
    },
    /// Stop all services in a running devx instance
    Down,
    /// Restart a specific service in a running devx instance
    Restart {
        /// Name of the service to restart
        service: String,
    },
    /// Show status of a running devx instance
    Status,
    /// Validate devx.toml and check infra
    Check,
    /// Trust the devx local CA in the system certificate store
    Trust,
    /// Tail logs from a running devx daemon
    Logs {
        /// Follow log output (like tail -f)
        #[arg(short = 'f', long = "follow")]
        follow: bool,
        /// Filter logs by service name
        #[arg(short = 's', long = "service")]
        service: Option<String>,
        /// Number of lines to show (default 50)
        #[arg(short = 'n', long = "lines", default_value = "50")]
        lines: usize,
    },
    /// Initialize a new devx.toml in the current directory
    Init,
}

fn main() -> Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("failed to install rustls crypto provider");

    let cli = Cli::parse();
    let project = cli.project;
    match cli.command {
        Commands::Up { daemon, services } => cmd_up(daemon, services, project),
        Commands::Down => cmd_down(project),
        Commands::Restart { service } => cmd_restart(service, project),
        Commands::Status => cmd_status(project),
        Commands::Check => cmd_check(),
        Commands::Trust => cmd_trust(),
        Commands::Logs {
            follow,
            service,
            lines,
        } => cmd_logs(follow, service, lines, project),
        Commands::Init => init::cmd_init(),
    }
}

fn find_project_root() -> Result<PathBuf> {
    let mut dir = std::env::current_dir()?;
    loop {
        if dir.join("devx.toml").exists() {
            return Ok(dir);
        }
        if !dir.pop() {
            bail!("devx.toml not found in current directory or any parent");
        }
    }
}

fn load_project_config() -> Result<(PathBuf, DevxConfig)> {
    let project_root = find_project_root()?;
    let config = DevxConfig::load(&project_root.join("devx.toml"))?;
    Ok((project_root, config))
}

fn service_filter(services: Vec<String>) -> Option<Vec<String>> {
    if services.is_empty() {
        None
    } else {
        Some(services)
    }
}

fn cmd_up(daemon_mode: bool, services: Vec<String>, project: Option<String>) -> Result<()> {
    let (project_root, config) = load_project_config()?;
    let project_id = resolve_project_id(&config, project.as_deref());

    if daemon::is_running(&project_id) {
        bail!(
            "devx is already running for '{}'. Use 'devx down' first.",
            project_id
        );
    }

    // Resolve the deterministic per-worktree port offset once, alongside the
    // project id. 0 in the primary checkout (ports unchanged byte-for-byte),
    // a stable non-zero value in a linked worktree.
    let port_offset = devx::ports::worktree_port_offset();

    if daemon_mode {
        cmd_up_daemon(config, project_root, services, project_id, port_offset)
    } else {
        cmd_up_tui(config, project_root, services, project_id, port_offset)
    }
}

fn cmd_up_tui(
    config: DevxConfig,
    project_root: PathBuf,
    services: Vec<String>,
    project_id: String,
    port_offset: u16,
) -> Result<()> {
    let (event_tx, event_rx) = mpsc::channel(8192);

    let project_id_cleanup = project_id.clone();
    let mut orchestrator = Orchestrator::new(
        config,
        project_root.clone(),
        event_tx,
        project_id.clone(),
        port_offset,
    );
    let service_names = orchestrator.service_names();

    let filter = service_filter(services);

    let runtime = tokio::runtime::Runtime::new()?;

    let cmd_tx = orchestrator.cmd_sender();

    // Spawn orchestrator start + command loop in the background
    let filter_clone = filter.clone();
    runtime.spawn(async move {
        let filter_ref = filter_clone.as_deref();
        if let Err(e) = orchestrator.start(filter_ref).await {
            eprintln!("orchestrator error: {}", e);
            return;
        }
        orchestrator.run_loop().await;
    });

    let branch = devx::git::current_branch();

    let mut terminal = ratatui::init();
    let mut app = App::new(project_id, service_names, branch, cmd_tx, event_rx);

    let result = runtime.block_on(app.run(&mut terminal));

    ratatui::restore();

    // Clean up the control socket
    control::cleanup(&project_id_cleanup);

    // Shutdown runtime (drops orchestrator, kills processes via kill_on_drop)
    runtime.shutdown_timeout(std::time::Duration::from_secs(2));

    result
}

fn cmd_up_daemon(
    config: DevxConfig,
    project_root: PathBuf,
    services: Vec<String>,
    project_id: String,
    port_offset: u16,
) -> Result<()> {
    // Parent prints PID and exits; child continues past this point
    daemon::daemonize(&project_id)?;

    let log_file_path = daemon::log_path(&project_id);
    let project_id_cleanup = project_id.clone();

    let (event_tx, mut event_rx) = mpsc::channel(8192);
    let mut orchestrator = Orchestrator::new(
        config,
        project_root.clone(),
        event_tx,
        project_id.clone(),
        port_offset,
    );
    let cmd_tx = orchestrator.cmd_sender();
    let filter = service_filter(services);

    let runtime = tokio::runtime::Runtime::new()?;

    let filter_clone = filter.clone();
    runtime.spawn(async move {
        let filter_ref = filter_clone.as_deref();
        if let Err(e) = orchestrator.start(filter_ref).await {
            eprintln!("orchestrator error: {}", e);
            return;
        }
        orchestrator.run_loop().await;
    });

    runtime.block_on(async {
        let mut log_file = match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_file_path)
        {
            Ok(f) => f,
            Err(e) => {
                eprintln!("failed to open log file: {}", e);
                return;
            }
        };

        let ts = daemon::format_timestamp(SystemTime::now());
        let _ = writeln!(log_file, "[{}] [devx] daemon started (pid {})", ts, std::process::id());
        let _ = log_file.flush();

        let mut sigterm = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(s) => s,
            Err(e) => {
                let ts = daemon::format_timestamp(SystemTime::now());
                let _ = writeln!(log_file, "[{}] [devx] failed to register SIGTERM handler: {}", ts, e);
                let _ = log_file.flush();
                return;
            }
        };

        loop {
            tokio::select! {
                event = event_rx.recv() => {
                    match event {
                        Some(evt) => {
                            write_event_to_log(&mut log_file, &evt);
                            if matches!(evt, DevxEvent::ControlShutdown) {
                                let ts = daemon::format_timestamp(SystemTime::now());
                                let _ = writeln!(log_file, "[{}] [devx] received shutdown command", ts);
                                let _ = log_file.flush();
                                break;
                            }
                        }
                        None => break,
                    }
                }
                _ = sigterm.recv() => {
                    let ts = daemon::format_timestamp(SystemTime::now());
                    let _ = writeln!(log_file, "[{}] [devx] received SIGTERM, shutting down", ts);
                    let _ = log_file.flush();
                    break;
                }
            }
        }

        // Tell orchestrator to gracefully stop all services (SIGTERM → 5s → SIGKILL)
        let _ = cmd_tx.send(devx::orchestrator::OrchestratorCommand::Shutdown).await;

        let ts = daemon::format_timestamp(SystemTime::now());
        let _ = writeln!(log_file, "[{}] [devx] daemon stopped", ts);
        let _ = log_file.flush();
    });

    control::cleanup(&project_id_cleanup);
    daemon::cleanup_pid(&project_id_cleanup);
    runtime.shutdown_timeout(std::time::Duration::from_secs(2));

    Ok(())
}

fn write_event_to_log(log_file: &mut std::fs::File, event: &DevxEvent) {
    // Skip events that produce no output
    if matches!(event, DevxEvent::Tick | DevxEvent::ControlShutdown) {
        return;
    }

    let ts = daemon::format_timestamp(SystemTime::now());
    match event {
        DevxEvent::LogLine {
            service,
            line,
            is_stderr,
        } => {
            if *is_stderr {
                let _ = writeln!(log_file, "[{}] [{}] [stderr] {}", ts, service, line);
            } else {
                let _ = writeln!(log_file, "[{}] [{}] {}", ts, service, line);
            }
        }
        DevxEvent::StateChange { service, state, .. } => {
            let _ = writeln!(log_file, "[{}] [{}] state: {}", ts, service, state.label());
        }
        DevxEvent::ProxyBound {
            service,
            proxy_port,
            target_port,
        } => {
            let _ = writeln!(
                log_file,
                "[{}] [{}] proxy :{} -> :{}",
                ts, service, proxy_port, target_port
            );
        }
        DevxEvent::VhostBound { port, domains, tls } => {
            let domain_list: Vec<&str> = domains.iter().map(|(d, _)| d.as_str()).collect();
            let proto = if *tls { "https" } else { "http" };
            let _ = writeln!(
                log_file,
                "[{}] [devx] vhost {} :{} domains: {}",
                ts,
                proto,
                port,
                domain_list.join(", ")
            );
        }
        DevxEvent::FileChanged { service } => {
            let _ = writeln!(log_file, "[{}] [{}] file changed, restarting", ts, service);
        }
        DevxEvent::ConfigReloaded { diff } => {
            let mut parts = Vec::new();
            for name in &diff.added {
                parts.push(format!("added {}", name));
            }
            for name in &diff.removed {
                parts.push(format!("removed {}", name));
            }
            for name in &diff.changed {
                parts.push(format!("changed {}", name));
            }
            let summary = if parts.is_empty() {
                "no changes".to_string()
            } else {
                parts.join(", ")
            };
            let _ = writeln!(log_file, "[{}] [devx] config reloaded: {}", ts, summary);
        }
        DevxEvent::AllStarted => {
            let _ = writeln!(log_file, "[{}] [devx] all services started", ts);
        }
        DevxEvent::ControlRestart { service } => {
            let _ = writeln!(log_file, "[{}] [devx] control: restart {}", ts, service);
        }
        DevxEvent::Tick | DevxEvent::ControlShutdown => unreachable!(),
    }
    let _ = log_file.flush();
}

fn cmd_down(project: Option<String>) -> Result<()> {
    let (_project_root, config) = load_project_config()?;
    let project_id = resolve_project_id(&config, project.as_deref());

    let rt = tokio::runtime::Runtime::new()?;
    let response = rt.block_on(control::send_command(&project_id, r#"{"cmd":"shutdown"}"#))?;

    if response.contains("\"ok\"") {
        println!("devx stopped");
    } else {
        eprintln!("unexpected response: {}", response.trim());
    }

    // Clean up PID file if it exists (daemon mode)
    daemon::cleanup_pid(&project_id);

    // Show log file location if it exists
    let log = daemon::log_path(&project_id);
    if log.exists() {
        println!("  logs: {}", log.display());
    }

    Ok(())
}

fn cmd_restart(service: String, project: Option<String>) -> Result<()> {
    let (_project_root, config) = load_project_config()?;
    let project_id = resolve_project_id(&config, project.as_deref());

    let cmd = serde_json::json!({"cmd": "restart", "service": service}).to_string();

    let rt = tokio::runtime::Runtime::new()?;
    let response = rt.block_on(control::send_command(&project_id, &cmd))?;

    if response.contains("\"ok\"") {
        println!("{} restarted", service);
    } else {
        eprintln!("unexpected response: {}", response.trim());
    }

    Ok(())
}

fn cmd_status(project: Option<String>) -> Result<()> {
    let (_project_root, config) = load_project_config()?;
    let project_id = resolve_project_id(&config, project.as_deref());

    let socket = control::socket_path(&project_id);
    if !socket.exists() {
        println!("devx is not running for '{}'", project_id);
        return Ok(());
    }

    let rt = tokio::runtime::Runtime::new()?;
    let response = match rt.block_on(control::send_command(&project_id, r#"{"cmd":"status"}"#)) {
        Ok(r) => r,
        Err(_) => {
            // Socket exists but nobody is listening — stale from a crash
            println!("devx appears to have crashed (stale socket)");
            println!("  cleaning up: {}", socket.display());
            control::cleanup(&project_id);
            daemon::cleanup_pid(&project_id);
            return Ok(());
        }
    };

    let status: serde_json::Value = serde_json::from_str(response.trim())?;

    if let Some(error) = status.get("error") {
        eprintln!("error: {}", error);
        return Ok(());
    }

    // Show daemon info if PID file exists
    if let Some(pid) = daemon::read_pid(&project_id) {
        println!("devx daemon running (pid {})", pid);
        println!("  logs: {}", daemon::log_path(&project_id).display());
        println!();
    }

    // Print service table
    if let Some(services) = status.get("services").and_then(|s| s.as_array()) {
        println!(
            "{:<20} {:<12} {:<8} {:<12} UPTIME",
            "SERVICE", "STATE", "PORT", "PROXY"
        );
        println!("{}", "-".repeat(60));

        for svc in services {
            let name = svc.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            let state = svc.get("state").and_then(|v| v.as_str()).unwrap_or("?");
            let port = svc
                .get("port")
                .and_then(|v| v.as_u64())
                .map(|p| p.to_string())
                .unwrap_or_else(|| "-".to_string());
            let proxy_port = svc
                .get("proxy_port")
                .and_then(|v| v.as_u64())
                .map(|p| p.to_string())
                .unwrap_or_else(|| "-".to_string());
            let uptime = svc
                .get("uptime_secs")
                .and_then(|v| v.as_u64())
                .map(format_uptime)
                .unwrap_or_else(|| "-".to_string());

            println!(
                "{:<20} {:<12} {:<8} {:<12} {}",
                name, state, port, proxy_port, uptime
            );
        }
    }

    Ok(())
}

fn format_uptime(secs: u64) -> String {
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m{}s", secs / 60, secs % 60)
    } else {
        format!("{}h{}m", secs / 3600, (secs % 3600) / 60)
    }
}

fn cmd_logs(
    follow: bool,
    service_filter: Option<String>,
    lines: usize,
    project: Option<String>,
) -> Result<()> {
    let (_project_root, config) = load_project_config()?;
    let project_id = resolve_project_id(&config, project.as_deref());

    let log_file_path = daemon::log_path(&project_id);
    if !log_file_path.exists() {
        bail!(
            "no log file found for '{}' (expected {}). Is the daemon running?",
            project_id,
            log_file_path.display()
        );
    }

    let filter_pattern = service_filter.as_ref().map(|svc| format!("[{}]", svc));
    let matches_filter =
        |line: &str| -> bool { filter_pattern.as_ref().is_none_or(|p| line.contains(p)) };

    // Stream through file keeping only the last N matching lines (avoids loading entire file)
    let file = std::fs::File::open(&log_file_path)?;
    let reader = std::io::BufReader::new(file);
    let mut tail: VecDeque<String> = VecDeque::with_capacity(lines);
    for line in reader.lines() {
        let line = line?;
        if matches_filter(&line) {
            if tail.len() >= lines {
                tail.pop_front();
            }
            tail.push_back(line);
        }
    }

    for line in &tail {
        println!("{}", line);
    }

    if !follow {
        return Ok(());
    }

    if !daemon::is_running(&project_id) {
        println!("(daemon is not running, cannot follow)");
        return Ok(());
    }

    let mut file = std::fs::File::open(&log_file_path)?;
    file.seek(std::io::SeekFrom::End(0))?;
    let mut reader = std::io::BufReader::new(file);

    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => {
                std::thread::sleep(std::time::Duration::from_millis(200));
                if !daemon::is_running(&project_id) {
                    println!("(daemon stopped)");
                    break;
                }
            }
            Ok(_) => {
                let line = line.trim_end();
                if matches_filter(line) {
                    println!("{}", line);
                }
            }
            Err(e) => {
                bail!("error reading log file: {}", e);
            }
        }
    }

    Ok(())
}

fn cmd_check() -> Result<()> {
    let (project_root, config) = load_project_config()?;

    let service_count = config.services.len();
    println!("devx.toml: valid ({} services)", service_count);

    if let Some(infra_cfg) = &config.infra {
        let compose_path = project_root.join(&infra_cfg.compose);
        match infra::check_compose(&compose_path) {
            Ok(()) => println!("infra: all containers running"),
            Err(e) => println!("infra: {}", e),
        }
    }

    Ok(())
}

fn cmd_trust() -> Result<()> {
    tls::trust_ca()
}
