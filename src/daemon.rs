use std::fs;
use std::io::Write;
use std::path::PathBuf;

use anyhow::{bail, Result};

/// Returns the PID file path for a given project name.
pub fn pid_path(project_name: &str) -> PathBuf {
    PathBuf::from(format!("/tmp/devx-{}.pid", project_name))
}

/// Returns the log file path for a given project name.
pub fn log_path(project_name: &str) -> PathBuf {
    PathBuf::from(format!("/tmp/devx-{}.log", project_name))
}

/// Read the PID from the PID file. Returns None if the file does not exist
/// or the contents cannot be parsed as a u32.
pub fn read_pid(project_name: &str) -> Option<u32> {
    let path = pid_path(project_name);
    let content = fs::read_to_string(path).ok()?;
    content.trim().parse::<u32>().ok()
}

/// Check if a daemon is running for the given project. Returns true if the
/// PID file exists AND the process is alive (via kill(pid, 0)).
pub fn is_running(project_name: &str) -> bool {
    match read_pid(project_name) {
        Some(pid) => {
            // SAFETY: kill with signal 0 is a process-existence probe with no
            // side effects.
            unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
        }
        None => false,
    }
}

/// Remove the PID file for a given project.
pub fn cleanup_pid(project_name: &str) {
    let path = pid_path(project_name);
    let _ = fs::remove_file(path);
}

/// Fork the current process and daemonize. The parent process prints the child
/// PID and exits. The child process continues execution after this function
/// returns.
///
/// Steps:
/// 1. fork() — parent gets child PID, prints it, exits
/// 2. Child calls setsid() to become session leader
/// 3. Redirects stdout/stderr to the log file
/// 4. Writes PID to the PID file
pub fn daemonize(project_name: &str) -> Result<()> {
    let log = log_path(project_name);
    let pid_file = pid_path(project_name);

    // SAFETY: fork() creates a new process. The parent returns the child PID,
    // the child returns 0. This is standard Unix daemonization.
    let pid = unsafe { libc::fork() };

    if pid < 0 {
        bail!("fork() failed: {}", std::io::Error::last_os_error());
    }

    if pid > 0 {
        // Parent process — print the child PID and exit
        println!("devx daemon started (pid {})", pid);
        println!("  logs: {}", log.display());
        std::process::exit(0);
    }

    // SAFETY: setsid() detaches from the controlling terminal
    if unsafe { libc::setsid() } == -1 {
        bail!("setsid() failed: {}", std::io::Error::last_os_error());
    }

    let log_file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)?;

    let log_fd = std::os::unix::io::AsRawFd::as_raw_fd(&log_file);

    // SAFETY: dup2 redirects stdout/stderr to our log file
    unsafe {
        libc::dup2(log_fd, libc::STDOUT_FILENO);
        libc::dup2(log_fd, libc::STDERR_FILENO);
    }
    drop(log_file); // stdout/stderr now own the fd

    let child_pid = std::process::id();
    let mut f = fs::File::create(&pid_file)?;
    writeln!(f, "{}", child_pid)?;

    Ok(())
}

/// Format a SystemTime as `YYYY-MM-DD HH:MM:SS` in local time.
/// Uses libc localtime_r to avoid adding dependencies.
pub fn format_timestamp(time: std::time::SystemTime) -> String {
    let duration = time
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs() as libc::time_t;

    // SAFETY: localtime_r is thread-safe (unlike localtime) and writes to
    // the provided tm struct. We zero-initialize it first.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe {
        libc::localtime_r(&secs, &mut tm);
    }

    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pid_path() {
        assert_eq!(pid_path("myapp"), PathBuf::from("/tmp/devx-myapp.pid"));
    }

    #[test]
    fn test_log_path() {
        assert_eq!(log_path("myapp"), PathBuf::from("/tmp/devx-myapp.log"));
    }

    #[test]
    fn test_read_pid_missing() {
        // Nonexistent project should return None
        assert_eq!(read_pid("__nonexistent_test_project_xyz__"), None);
    }

    #[test]
    fn test_is_running_missing() {
        assert!(!is_running("__nonexistent_test_project_xyz__"));
    }

    #[test]
    fn test_format_timestamp() {
        // Just verify it returns a properly formatted string (length and separators)
        let ts = format_timestamp(std::time::SystemTime::now());
        assert_eq!(ts.len(), 19); // "YYYY-MM-DD HH:MM:SS"
        assert_eq!(&ts[4..5], "-");
        assert_eq!(&ts[7..8], "-");
        assert_eq!(&ts[10..11], " ");
        assert_eq!(&ts[13..14], ":");
        assert_eq!(&ts[16..17], ":");
    }
}
