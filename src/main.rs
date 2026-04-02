use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use tokio::sync::mpsc;

use devx::config::DevxConfig;
use devx::control;
use devx::infra;
use devx::orchestrator::Orchestrator;
use devx::tls;
use devx::tui::App;

#[derive(Parser)]
#[command(name = "devx", about = "Local development orchestrator")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start all services
    Up {
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
    /// Validate devx.toml and check infra
    Check,
    /// Show whether devx is running for this project
    Status,
    /// Trust the devx local CA in the system certificate store
    Trust,
}

fn main() -> Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("failed to install rustls crypto provider");

    let cli = Cli::parse();
    match cli.command {
        Commands::Up { services } => cmd_up(services),
        Commands::Down => cmd_down(),
        Commands::Restart { service } => cmd_restart(service),
        Commands::Check => cmd_check(),
        Commands::Status => cmd_status(),
        Commands::Trust => cmd_trust(),
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

fn cmd_up(services: Vec<String>) -> Result<()> {
    let project_root = find_project_root()?;
    let config = DevxConfig::load(&project_root.join("devx.toml"))?;

    let (event_tx, event_rx) = mpsc::channel(8192);

    let project_name = config.project.name.clone();
    let project_name_cleanup = project_name.clone();
    let mut orchestrator = Orchestrator::new(config, project_root.clone(), event_tx);
    let service_names = orchestrator.service_names();

    let filter: Option<Vec<String>> = if services.is_empty() {
        None
    } else {
        Some(services)
    };

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
    let mut app = App::new(project_name, service_names, branch, cmd_tx, event_rx);

    let result = runtime.block_on(app.run(&mut terminal));

    ratatui::restore();

    // Clean up the control socket
    control::cleanup(&project_name_cleanup);

    // Shutdown runtime (drops orchestrator, kills processes via kill_on_drop)
    runtime.shutdown_timeout(std::time::Duration::from_secs(2));

    result
}

fn cmd_down() -> Result<()> {
    let project_root = find_project_root()?;
    let config = DevxConfig::load(&project_root.join("devx.toml"))?;
    let project_name = config.project.name;

    let rt = tokio::runtime::Runtime::new()?;
    let response = rt.block_on(control::send_command(
        &project_name,
        r#"{"cmd":"shutdown"}"#,
    ))?;

    if response.contains("\"ok\"") {
        println!("devx stopped");
    } else {
        eprintln!("unexpected response: {}", response.trim());
    }

    Ok(())
}

fn cmd_restart(service: String) -> Result<()> {
    let project_root = find_project_root()?;
    let config = DevxConfig::load(&project_root.join("devx.toml"))?;
    let project_name = config.project.name;

    let cmd = serde_json::json!({"cmd": "restart", "service": service}).to_string();

    let rt = tokio::runtime::Runtime::new()?;
    let response = rt.block_on(control::send_command(&project_name, &cmd))?;

    if response.contains("\"ok\"") {
        println!("{} restarted", service);
    } else {
        eprintln!("unexpected response: {}", response.trim());
    }

    Ok(())
}

fn cmd_status() -> Result<()> {
    let project_root = find_project_root()?;
    let config = DevxConfig::load(&project_root.join("devx.toml"))?;
    let sock = control::socket_path(&config.project.name);

    if sock.exists() {
        println!("devx is running (project: {})", config.project.name);
        println!("socket: {}", sock.display());
        println!(
            "services: {}",
            config
                .services
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        );
    } else {
        println!("devx is not running (project: {})", config.project.name);
    }

    Ok(())
}

fn cmd_check() -> Result<()> {
    let project_root = find_project_root()?;
    let config = DevxConfig::load(&project_root.join("devx.toml"))?;

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
