use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize)]
struct ComposeService {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "State")]
    state: String,
}

/// Start infra containers with `docker compose up -d`, then verify they're running.
pub fn ensure_compose(compose_path: &Path) -> Result<()> {
    let path_str = compose_path
        .to_str()
        .context("compose path is not valid UTF-8")?;

    // Start containers
    let up = std::process::Command::new("docker")
        .args(["compose", "-f", path_str, "up", "-d"])
        .output()
        .context("failed to run `docker compose up -d` — is Docker running?")?;

    if !up.status.success() {
        let stderr = String::from_utf8_lossy(&up.stderr);
        bail!("`docker compose up -d` failed:\n{}", stderr.trim());
    }

    // Verify all containers are running
    check_compose(compose_path)
}

pub fn check_compose(compose_path: &Path) -> Result<()> {
    let output = std::process::Command::new("docker")
        .args([
            "compose",
            "-f",
            compose_path
                .to_str()
                .context("compose path is not valid UTF-8")?,
            "ps",
            "--format",
            "json",
        ])
        .output()
        .context("failed to run `docker compose ps` — is Docker running?")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "`docker compose ps` failed.\n{}\n\nIs Docker running and is the compose file valid?",
            stderr.trim()
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut not_running: Vec<String> = Vec::new();

    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // docker compose ps --format json outputs one JSON object per line
        match serde_json::from_str::<ComposeService>(line) {
            Ok(svc) => {
                if svc.state != "running" {
                    not_running.push(format!("  - {} ({})", svc.name, svc.state));
                }
            }
            Err(_) => {
                // Skip lines that don't parse (e.g. warnings)
            }
        }
    }

    if !not_running.is_empty() {
        bail!(
            "The following infra services are not running:\n{}\n\nRun `docker compose up -d` to start them.",
            not_running.join("\n")
        );
    }

    Ok(())
}
