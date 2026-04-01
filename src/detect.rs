use std::path::Path;

/// Detected framework with port injection strategy.
pub enum Framework {
    /// Vite/Astro/React Router — use --port and --host flags
    Vite,
    /// Next.js — uses PORT env var
    NextJs,
    /// Express/Node — uses PORT env var
    Node,
    /// Go (go run) — uses env var specified in config
    Go,
    /// Python (uvicorn/gunicorn) — uses --port flag
    Python,
    /// Unknown — don't inject anything
    Unknown,
}

impl Framework {
    /// Detect framework from the command string and working directory.
    pub fn detect(cmd: &str, work_dir: &Path) -> Self {
        // Check command patterns
        if cmd.contains("vite") || cmd.contains("astro") {
            return Framework::Vite;
        }
        if cmd.contains("next") {
            return Framework::NextJs;
        }
        if cmd.contains("uvicorn") || cmd.contains("gunicorn") {
            return Framework::Python;
        }
        if cmd.contains("go run") {
            return Framework::Go;
        }

        // Check package.json for framework hints.
        // Intentionally synchronous: this reads a small local file and is called
        // before the async spawn in process.rs, so blocking here is fine.
        let pkg_path = work_dir.join("package.json");
        if pkg_path.exists()
            && let Ok(content) = std::fs::read_to_string(&pkg_path)
        {
            if content.contains("\"vite\"") || content.contains("\"astro\"") {
                return Framework::Vite;
            }
            if content.contains("\"next\"") {
                return Framework::NextJs;
            }
            return Framework::Node;
        }

        Framework::Unknown
    }

    /// Modify the command to include port injection if needed.
    /// Returns None if the command already contains ${port} or the framework uses env vars.
    pub fn inject_port_flag(&self, cmd: &str, port: u16) -> Option<String> {
        // Don't inject if cmd already has port placeholder or explicit --port
        if cmd.contains("${port}") || cmd.contains("--port") {
            return None;
        }

        match self {
            Framework::Vite => Some(format!("{} --port {} --host", cmd, port)),
            Framework::Python => {
                // uvicorn uses --port, gunicorn uses -b
                if cmd.contains("uvicorn") {
                    Some(format!("{} --port {}", cmd, port))
                } else if cmd.contains("gunicorn") {
                    Some(format!("{} -b 127.0.0.1:{}", cmd, port))
                } else {
                    None
                }
            }
            // NextJs, Node, Go use PORT env var — handled by process.rs env injection
            Framework::NextJs | Framework::Node | Framework::Go | Framework::Unknown => None,
        }
    }

    /// Should we inject PORT as an env var?
    pub fn injects_port_env(&self) -> bool {
        matches!(self, Framework::NextJs | Framework::Node)
    }
}
