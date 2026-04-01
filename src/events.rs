use std::time::Instant;

use crate::config::ConfigDiff;

#[derive(Debug, Clone, PartialEq)]
pub enum ServiceState {
    Pending,
    Starting,
    Healthy,
    Unhealthy,
    Failed(String),
    Stopped,
}

impl ServiceState {
    pub fn symbol(&self) -> &str {
        match self {
            ServiceState::Stopped => "○",
            _ => "●",
        }
    }

    pub fn label(&self) -> &str {
        match self {
            ServiceState::Pending => "pending",
            ServiceState::Starting => "starting",
            ServiceState::Healthy => "ready",
            ServiceState::Unhealthy => "unhealthy",
            ServiceState::Failed(_) => "failed",
            ServiceState::Stopped => "stopped",
        }
    }
}

#[derive(Debug, Clone)]
pub enum DevxEvent {
    LogLine {
        service: String,
        line: String,
        is_stderr: bool,
    },
    StateChange {
        service: String,
        state: ServiceState,
        started_at: Option<Instant>,
    },
    ProxyBound {
        service: String,
        proxy_port: u16,
        target_port: u16,
    },
    VhostBound {
        port: u16,
        domains: Vec<(String, String)>, // (domain, service_name)
        tls: bool,
    },
    FileChanged {
        service: String,
    },
    ConfigReloaded {
        diff: ConfigDiff,
    },
    AllStarted,
    Tick,
    ControlShutdown,
    ControlRestart {
        service: String,
    },
}
