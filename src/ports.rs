use anyhow::{Context, Result};
use std::collections::HashSet;
use std::net::TcpListener;

#[derive(Debug, Clone)]
pub struct PortAllocation {
    pub preferred: Option<u16>,
    pub actual: u16,
    pub remapped: bool,
}

/// Stride between adjacent per-worktree port slots. Every preferred port in a
/// linked worktree is shifted by a multiple of this, so a service on base port
/// `P` lands on `P`, `P+10`, `P+20`, … depending on the worktree.
pub const WORKTREE_PORT_STRIDE: u16 = 10;

/// Number of distinct non-zero slots a linked worktree can occupy. A worktree's
/// offset is one of `{1·stride, 2·stride, …, SLOTS·stride}`; the primary
/// checkout always uses 0. Kept small so offset ports stay near the base (and
/// thus bookmarkable / Clerk-friendly on plain localhost).
pub const WORKTREE_PORT_SLOTS: u16 = 16;

/// Deterministic, stateless per-worktree preferred-port offset.
///
/// - **Primary checkout / non-git dir → 0.** Preferred ports are returned
///   byte-for-byte unchanged — this is the load-bearing backward-compat
///   invariant (`git::is_worktree()` is `false` for the primary checkout and
///   for any non-repo dir, errors swallowed).
/// - **Linked git worktree → a stable offset** in `{stride .. SLOTS·stride}`,
///   derived from the worktree directory basename (the same stable, path-based
///   key `git::worktree_suffix()` already feeds into the project id). Never 0,
///   so a worktree's preferred ports can never collide with the primary's.
///
/// Stateless by design: the same worktree path always derives the same offset
/// with no registry file to allocate, persist, or garbage-collect. On the rare
/// hash collision (two distinct worktrees → same slot) the preferred ports
/// match, and [`PortAllocator::allocate`]'s existing fall-back-to-free behavior
/// keeps both instances bootable (just one loses its stable URL that session).
pub fn worktree_port_offset() -> u16 {
    if !crate::git::is_worktree() {
        return 0;
    }
    crate::git::worktree_suffix()
        .map(|s| offset_for_suffix(&s))
        .unwrap_or(0)
}

/// Pure offset core: hash `suffix` into a non-zero slot and scale by the stride.
/// Split out from [`worktree_port_offset`] so it is unit-testable without git.
///
/// Uses FNV-1a rather than `DefaultHasher`: FNV is specified and stable across
/// platforms and Rust versions, which a bookmarkable URL requires.
pub fn offset_for_suffix(suffix: &str) -> u16 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325; // FNV offset basis
    for b in suffix.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3); // FNV prime
    }
    let slot = (hash % WORKTREE_PORT_SLOTS as u64) as u16 + 1; // 1..=SLOTS, never 0
    slot * WORKTREE_PORT_STRIDE
}

/// Apply [`worktree_port_offset`] to a single preferred port, saturating so a
/// port near `u16::MAX` can never wrap. With offset 0 (primary checkout) this
/// returns `port` unchanged.
pub fn apply_offset(port: u16, offset: u16) -> u16 {
    port.saturating_add(offset)
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

#[cfg(test)]
mod offset_tests {
    use super::*;

    #[test]
    fn backward_compat_zero_offset_is_byte_for_byte() {
        // Load-bearing invariant: in the primary checkout the offset is 0, and
        // every preferred port must come back UNCHANGED. If this ever fails,
        // existing checkouts silently move off their bookmarked ports.
        for port in [80u16, 3001, 3100, 9001, 9003, 65535] {
            assert_eq!(apply_offset(port, 0), port);
        }
    }

    #[test]
    fn offset_is_deterministic() {
        // Same worktree key -> same offset across calls (stateless + stable).
        assert_eq!(
            offset_for_suffix("new-world-wt-devx"),
            offset_for_suffix("new-world-wt-devx")
        );
    }

    #[test]
    fn offset_is_a_positive_multiple_of_stride() {
        // Sweep many plausible worktree basenames: every offset must be a
        // multiple of the stride, never 0 (so it can't alias the primary), and
        // within the SLOTS band.
        for i in 0..2000 {
            let suffix = format!("wt-{i}");
            let off = offset_for_suffix(&suffix);
            assert!(off >= WORKTREE_PORT_STRIDE, "offset {off} below one stride");
            assert!(
                off <= WORKTREE_PORT_SLOTS * WORKTREE_PORT_STRIDE,
                "offset {off} above the SLOTS band"
            );
            assert_eq!(
                off % WORKTREE_PORT_STRIDE,
                0,
                "offset {off} not a stride multiple"
            );
        }
    }

    #[test]
    fn distinct_suffixes_generally_get_distinct_slots() {
        // Not a guarantee (collisions degrade to fall-back-to-free), but the
        // hash should spread a handful of real-world worktree names across
        // different slots — otherwise the feature buys nothing in practice.
        let names = [
            "new-world",
            "new-world-wt-devx",
            "new-world-wt-domain-migration",
            "adoring-hofstadter-29bf34",
            "frosty-allen-bc8374",
        ];
        let offsets: std::collections::HashSet<u16> =
            names.iter().map(|n| offset_for_suffix(n)).collect();
        assert!(
            offsets.len() >= names.len() - 1,
            "expected near-distinct slots, got {offsets:?}"
        );
    }

    #[test]
    fn apply_offset_saturates_instead_of_wrapping() {
        // A preferred port near the ceiling must clamp, never wrap to a tiny
        // privileged port.
        assert_eq!(apply_offset(u16::MAX, WORKTREE_PORT_STRIDE), u16::MAX);
        assert_eq!(apply_offset(65530, 100), u16::MAX);
    }
}
