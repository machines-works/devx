use crate::config::DevxConfig;
use crate::git;

/// Sanitize an identity segment to be filesystem/socket-safe under
/// /tmp/devx-{id}.sock. Mirrors the branch_domain() rule in git.rs:62-73:
/// lowercase, then any char that is not alphanumeric or '-' becomes '-'.
fn sanitize(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// Resolve the effective project id used to key /tmp/devx-{id}.{sock,pid,log}
/// and the singleton guard. Resolved ONCE per invocation; every subcommand
/// must derive its instance-control id through this function so they agree.
///
/// Precedence (first match wins):
///   (a) explicit --project/-p flag         -> sanitize(flag)
///   (b) DEVX_PROJECT env var (non-empty)    -> sanitize(env)
///   (c) linked worktree (is_worktree==true) -> "{config name}-{sanitize(toplevel basename)}"
///   (d) fallback                            -> config.project.name VERBATIM (no sanitize)
pub fn resolve_project_id(config: &DevxConfig, flag: Option<&str>) -> String {
    // Delegate the pure precedence logic so it is unit-testable without git/env.
    let env = std::env::var("DEVX_PROJECT").ok();
    let worktree_suffix = if git::is_worktree() {
        git::worktree_suffix()
    } else {
        None
    };
    resolve_project_id_with(
        &config.project.name,
        flag,
        env.as_deref(),
        worktree_suffix.as_deref(),
    )
}

/// Pure precedence core — no git, no env, no I/O. Unit-tested directly.
pub fn resolve_project_id_with(
    config_name: &str,
    flag: Option<&str>,
    env: Option<&str>,
    worktree_suffix: Option<&str>,
) -> String {
    if let Some(p) = flag {
        // (a)
        return sanitize(p);
    }
    if let Some(e) = env {
        // (b)
        if !e.trim().is_empty() {
            return sanitize(e);
        }
    }
    if let Some(sfx) = worktree_suffix {
        // (c)
        return format!("{}-{}", config_name, sanitize(sfx));
    }
    config_name.to_string() // (d) byte-for-byte
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_wins() {
        assert_eq!(
            resolve_project_id_with("nw", Some("sharpi"), Some("shimizu"), Some("wt")),
            "sharpi"
        );
    }

    #[test]
    fn env_used_when_no_flag() {
        assert_eq!(
            resolve_project_id_with("nw", None, Some("shimizu"), Some("wt")),
            "shimizu"
        );
    }

    #[test]
    fn empty_env_ignored() {
        // Whitespace-only env falls through to the worktree suffix.
        assert_eq!(
            resolve_project_id_with("nw", None, Some("  "), Some("wt")),
            "nw-wt"
        );
        // Empty env with no worktree falls all the way through to the config name.
        assert_eq!(resolve_project_id_with("nw", None, Some(""), None), "nw");
    }

    #[test]
    fn backward_compat_no_sanitize_on_fallback() {
        // Load-bearing: path (d) must return the config name byte-for-byte,
        // with NO sanitization (mixed case is preserved).
        assert_eq!(resolve_project_id_with("MyApp", None, None, None), "MyApp");
    }

    #[test]
    fn worktree_derive() {
        assert_eq!(
            resolve_project_id_with("new-world", None, None, Some("nw-feature-x")),
            "new-world-nw-feature-x"
        );
    }

    #[test]
    fn precedence_chain() {
        // All four sources present -> flag wins.
        assert_eq!(
            resolve_project_id_with("nw", Some("a"), Some("b"), Some("c")),
            "a"
        );
        // Drop flag -> env wins.
        assert_eq!(
            resolve_project_id_with("nw", None, Some("b"), Some("c")),
            "b"
        );
        // Drop env -> worktree wins.
        assert_eq!(resolve_project_id_with("nw", None, None, Some("c")), "nw-c");
        // Drop worktree -> fallback to config name.
        assert_eq!(resolve_project_id_with("nw", None, None, None), "nw");
    }

    #[test]
    fn sanitize_rule() {
        assert_eq!(sanitize("feat/x_1"), "feat-x-1");
        assert_eq!(sanitize("API"), "api");
    }

    #[test]
    fn sanitize_collision_is_intentional() {
        // Two distinct inputs that collapse to the same id is the documented,
        // intentional behavior — assert it explicitly so a future change to the
        // sanitize rule is a conscious decision.
        assert_eq!(sanitize("feat_x"), sanitize("feat/x"));
        assert_eq!(sanitize("feat_x"), "feat-x");
    }
}
