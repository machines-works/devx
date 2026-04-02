use std::path::Path;

use anyhow::{Result, bail};

use crate::detect::Framework;

struct DetectedService {
    name: String,
    dir: String,
    cmd: String,
}

/// Scan subdirectories for recognizable project markers and return detected services.
fn scan_services(root: &Path) -> Vec<DetectedService> {
    let mut services = Vec::new();

    let entries = match std::fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return services,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        // Skip hidden directories
        let dir_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) if !n.starts_with('.') => n.to_string(),
            _ => continue,
        };

        // Skip common non-service directories
        if matches!(
            dir_name.as_str(),
            "node_modules" | "target" | "dist" | "build" | ".git" | "vendor"
        ) {
            continue;
        }

        let cmd = if path.join("encore.app").exists() {
            "encore run"
        } else if path.join("package.json").exists() {
            // Use detect module to refine: might be Next.js, Vite, etc.
            let framework = Framework::detect("npm run dev", &path);
            match framework {
                Framework::Vite => "npm run dev",
                Framework::NextJs => "npm run dev",
                _ => "npm run dev",
            }
        } else if path.join("Cargo.toml").exists() {
            "cargo run"
        } else if path.join("go.mod").exists() {
            "go run ."
        } else if path.join("pyproject.toml").exists() || path.join("requirements.txt").exists() {
            "python -m uvicorn main:app"
        } else {
            continue;
        };

        services.push(DetectedService {
            name: dir_name.clone(),
            dir: dir_name,
            cmd: cmd.to_string(),
        });
    }

    // Sort by name for deterministic output
    services.sort_by(|a, b| a.name.cmp(&b.name));
    services
}

/// Generate the contents of a devx.toml file.
fn generate_toml(project_name: &str, services: &[DetectedService]) -> String {
    let mut out = String::new();

    out.push_str(&format!("[project]\nname = \"{}\"\n", project_name));

    if services.is_empty() {
        out.push_str(
            r#"
# No services were auto-detected. Add your services below.
# Each service needs at minimum a `cmd`.
# See https://devx.machines.works for full documentation.

# [services.example]
# cmd = "npm run dev"
# dir = "example"
# port = 3000
# health = "http://localhost:${port}/health"
# domain = "example.localhost"
# depends_on = []
# watch = true
"#,
        );
    } else {
        out.push_str(
            "\n# Add services below. Each service needs at minimum a `cmd`.\n\
             # See https://devx.machines.works for full documentation.\n",
        );

        let mut port = 3000u16;
        for svc in services {
            out.push_str(&format!(
                "\n[services.{}]\ncmd = \"{}\"\ndir = \"{}\"\nport = {}\n\
                 # health = \"http://localhost:${{port}}/health\"\n\
                 # domain = \"{}.localhost\"\n\
                 # depends_on = []\n\
                 # watch = true\n",
                svc.name, svc.cmd, svc.dir, port, svc.name,
            ));
            port += 1000;
        }
    }

    out
}

/// Entry point for `devx init`.
pub fn cmd_init() -> Result<()> {
    let cwd = std::env::current_dir()?;
    let toml_path = cwd.join("devx.toml");

    if toml_path.exists() {
        bail!("devx.toml already exists in this directory");
    }

    let project_name = cwd
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("project")
        .to_string();

    let services = scan_services(&cwd);
    let content = generate_toml(&project_name, &services);

    std::fs::write(&toml_path, &content)?;

    let count = services.len();
    if count > 0 {
        println!(
            "Created devx.toml with {} service{} detected. Run `devx up` to start.",
            count,
            if count == 1 { "" } else { "s" }
        );
    } else {
        println!(
            "Created devx.toml with no services detected. Edit it to add your services, then run `devx up`."
        );
    }

    Ok(())
}
