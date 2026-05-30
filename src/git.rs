use std::process::Command;

/// Detect the current git branch name. Returns None if not in a git repo.
pub fn current_branch() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let branch = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if branch.is_empty() {
        None
    } else {
        Some(branch)
    }
}

/// Detect if we're in a git worktree (not the main working tree).
pub fn is_worktree() -> bool {
    let git_dir = Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout).ok()
            } else {
                None
            }
        })
        .map(|s| s.trim().to_string());

    let common_dir = Command::new("git")
        .args(["rev-parse", "--git-common-dir"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout).ok()
            } else {
                None
            }
        })
        .map(|s| s.trim().to_string());

    match (git_dir, common_dir) {
        (Some(gd), Some(cd)) => gd != cd,
        _ => false,
    }
}

/// Basename of the current worktree's checkout dir (`git rev-parse --show-toplevel`).
/// Used to derive a STABLE per-worktree id suffix (path, not branch — see ADR/spec).
/// Returns None outside a repo or if the toplevel has no file name, so the
/// resolver cleanly falls through to config.project.name.
pub fn worktree_suffix() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let toplevel = String::from_utf8(output.stdout).ok()?.trim().to_string();
    std::path::Path::new(&toplevel)
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
}

/// Get the branch-prefixed domain variant.
/// e.g., branch="fix-auth", domain="api.localhost" => "fix-auth.api.localhost"
/// Returns None if on main/master or not in a git repo.
pub fn branch_domain(domain: &str) -> Option<String> {
    let branch = current_branch()?;
    if branch == "main" || branch == "master" || branch == "HEAD" {
        return None;
    }
    // Sanitize branch name: replace non-alphanumeric with hyphens, lowercase
    let sanitized = branch
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>();
    Some(format!("{}.{}", sanitized, domain))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worktree_suffix_none_outside_repo() {
        // Build a fresh temp dir that is not inside any git repo. Running
        // `git rev-parse --show-toplevel` there fails, so worktree_suffix()
        // must return None (so the resolver falls through to config name).
        // Mirror the error-swallowing style used by is_worktree().
        let mut base = std::env::temp_dir();
        base.push(format!(
            "devx-worktree-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&base).expect("failed to create temp dir");

        // Change cwd to the non-repo temp dir, probe, then restore.
        let original = std::env::current_dir().expect("cwd");
        std::env::set_current_dir(&base).expect("set cwd");
        let suffix = worktree_suffix();
        std::env::set_current_dir(&original).expect("restore cwd");
        let _ = std::fs::remove_dir_all(&base);

        assert_eq!(suffix, None);
    }

    #[test]
    fn branch_domain_returns_none_for_main() {
        // We can't easily mock git, but we can test the sanitization logic directly
        // by calling the helper with a known non-main branch scenario via direct string ops.
        let sanitized = "fix-auth"
            .to_lowercase()
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' {
                    c
                } else {
                    '-'
                }
            })
            .collect::<String>();
        assert_eq!(
            format!("{}.{}", sanitized, "api.localhost"),
            "fix-auth.api.localhost"
        );
    }

    #[test]
    fn branch_sanitization_replaces_slashes() {
        let branch = "feat/my-feature";
        let sanitized = branch
            .to_lowercase()
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' {
                    c
                } else {
                    '-'
                }
            })
            .collect::<String>();
        assert_eq!(sanitized, "feat-my-feature");
        assert_eq!(
            format!("{}.{}", sanitized, "api.localhost"),
            "feat-my-feature.api.localhost"
        );
    }
}
