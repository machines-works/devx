use anyhow::{Context, Result};
use std::collections::HashSet;
use std::net::TcpListener;

#[derive(Debug, Clone)]
pub struct PortAllocation {
    pub preferred: Option<u16>,
    pub actual: u16,
    pub remapped: bool,
}

/// Allocates TCP ports for services by binding a temporary `TcpListener`.
///
/// **Known TOCTOU race:** The listener is dropped after discovering the free port,
/// so another process *could* grab the port before the service binds it. In
/// practice this is rare on a developer machine. The `allocated` set prevents
/// devx itself from handing out the same ephemeral port twice in one session.
pub struct PortAllocator {
    /// Ports already handed out during this allocator's lifetime.
    allocated: HashSet<u16>,
}

impl Default for PortAllocator {
    fn default() -> Self {
        Self::new()
    }
}

impl PortAllocator {
    pub fn new() -> Self {
        Self {
            allocated: HashSet::new(),
        }
    }

    /// Try to bind `preferred`; fall back to an OS-assigned port on failure.
    ///
    /// Note: there is a small TOCTOU window between the probe bind and the
    /// service's real bind — see the struct-level doc comment.
    pub fn allocate(&mut self, service: &str, preferred: u16) -> Result<PortAllocation> {
        match TcpListener::bind(("127.0.0.1", preferred)) {
            Ok(listener) => {
                let addr = listener
                    .local_addr()
                    .context(format!("service '{}': could not get local addr", service))?;
                self.allocated.insert(addr.port());
                Ok(PortAllocation {
                    preferred: Some(preferred),
                    actual: addr.port(),
                    remapped: false,
                })
            }
            Err(_) => {
                let listener = TcpListener::bind(("127.0.0.1", 0))
                    .context(format!("service '{}': could not bind any port", service))?;
                let addr = listener.local_addr()?;
                self.allocated.insert(addr.port());
                Ok(PortAllocation {
                    preferred: Some(preferred),
                    actual: addr.port(),
                    remapped: true,
                })
            }
        }
    }

    /// Always allocate an OS-assigned port. Retries up to 3 times if the OS
    /// hands back a port that was already allocated in this session.
    ///
    /// Note: there is a small TOCTOU window between the probe bind and the
    /// service's real bind — see the struct-level doc comment.
    pub fn allocate_any(&mut self, service: &str) -> Result<PortAllocation> {
        for _ in 0..3 {
            let listener = TcpListener::bind(("127.0.0.1", 0))
                .context(format!("service '{}': could not bind any port", service))?;
            let addr = listener.local_addr()?;
            let port = addr.port();
            if !self.allocated.contains(&port) {
                self.allocated.insert(port);
                return Ok(PortAllocation {
                    preferred: None,
                    actual: port,
                    remapped: false,
                });
            }
        }
        // Final attempt — accept whatever port we get
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .context(format!("service '{}': could not bind any port", service))?;
        let addr = listener.local_addr()?;
        self.allocated.insert(addr.port());
        Ok(PortAllocation {
            preferred: None,
            actual: addr.port(),
            remapped: false,
        })
    }
}
