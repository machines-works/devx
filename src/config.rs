use anyhow::{bail, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize, Default)]
pub struct ProxyConfig {
    #[serde(default = "default_true")]
    pub tls: bool,
}

#[derive(Debug, Deserialize)]
pub struct DevxConfig {
    pub project: ProjectConfig,
    pub infra: Option<InfraConfig>,
    pub proxy: Option<ProxyConfig>,
    #[serde(default)]
    pub services: HashMap<String, ServiceConfig>,
}

#[derive(Debug, Deserialize)]
pub struct ProjectConfig {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct InfraConfig {
    pub compose: String,
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct ServiceConfig {
    pub cmd: String,
    pub dir: Option<String>,
    pub port: Option<u16>,
    pub health: Option<String>,
    pub domain: Option<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default = "default_true")]
    pub watch: bool,
}

impl DevxConfig {
    pub fn tls_enabled(&self) -> bool {
        self.proxy.as_ref().is_none_or(|p| p.tls)
    }

    pub fn parse(s: &str) -> Result<Self> {
        let config: DevxConfig = toml::from_str(s)?;
        config.validate()?;
        Ok(config)
    }

    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        Self::parse(&content)
    }

    pub fn diff(&self, other: &DevxConfig) -> ConfigDiff {
        let mut added = Vec::new();
        let mut removed = Vec::new();
        let mut changed = Vec::new();
        let mut unchanged = Vec::new();

        for (name, old_svc) in &self.services {
            match other.services.get(name) {
                Some(new_svc) if old_svc == new_svc => unchanged.push(name.clone()),
                Some(_) => changed.push(name.clone()),
                None => removed.push(name.clone()),
            }
        }

        for name in other.services.keys() {
            if !self.services.contains_key(name) {
                added.push(name.clone());
            }
        }

        ConfigDiff {
            added,
            removed,
            changed,
            unchanged,
        }
    }

    fn validate(&self) -> Result<()> {
        if self.services.is_empty() {
            bail!("config must define at least one service");
        }
        for (name, svc) in &self.services {
            if svc.cmd.trim().is_empty() {
                bail!("service '{}' has an empty cmd", name);
            }
            for dep in &svc.depends_on {
                if !self.services.contains_key(dep) {
                    bail!(
                        "service '{}' depends_on '{}' which does not exist",
                        name,
                        dep
                    );
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct ConfigDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub changed: Vec<String>,
    pub unchanged: Vec<String>,
}

/// Replace `${port}` with the service's actual port and `${proxy:NAME}` with the
/// proxy port for NAME.
pub fn interpolate(
    template: &str,
    service_name: &str,
    actual_ports: &HashMap<String, u16>,
    proxy_ports: &HashMap<String, u16>,
) -> String {
    let mut result = template.to_string();

    // Replace ${port} with this service's actual port
    if let Some(&port) = actual_ports.get(service_name) {
        result = result.replace("${port}", &port.to_string());
    }

    // Replace ${proxy:NAME} patterns
    let mut output = String::new();
    let mut remaining = result.as_str();
    while let Some(start) = remaining.find("${proxy:") {
        output.push_str(&remaining[..start]);
        let after = &remaining[start + "${proxy:".len()..];
        if let Some(end) = after.find('}') {
            let name = &after[..end];
            if let Some(&port) = proxy_ports.get(name) {
                output.push_str(&port.to_string());
            } else {
                // leave the placeholder intact
                output.push_str("${proxy:");
                output.push_str(name);
                output.push('}');
            }
            remaining = &after[end + 1..];
        } else {
            // malformed placeholder — emit as-is
            output.push_str("${proxy:");
            remaining = after;
        }
    }
    output.push_str(remaining);
    output
}
