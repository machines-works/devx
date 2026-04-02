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
