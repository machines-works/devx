//! Integration tests for the devx CLI.
//!
//! These tests invoke the real `devx` binary against real config files,
//! real sockets, and real processes. No mocks.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};
use std::{fs, thread};

use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn devx_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_devx"))
}

fn devx(dir: &Path) -> Command {
    let mut cmd = Command::new(devx_bin());
    cmd.current_dir(dir);
    cmd
}

/// Create a temp directory with a devx.toml. Returns (TempDir, project_name).
/// The project name includes a random suffix to avoid socket collisions.
fn setup_project(toml_content: &str) -> (TempDir, String) {
    let dir = TempDir::new().expect("failed to create temp dir");
    fs::write(dir.path().join("devx.toml"), toml_content).expect("failed to write devx.toml");
    // Extract project name from the toml
    let name = toml_content
        .lines()
        .find(|l| l.starts_with("name"))
        .and_then(|l| l.split('"').nth(1))
        .expect("devx.toml must have a project name")
        .to_string();
    (dir, name)
}

/// Generate a unique project name for test isolation.
fn unique_name(base: &str) -> String {
    let id: u32 = rand_u32();
    format!("{}-{}", base, id)
}

/// Poor-man's random u32 from system time nanos + process id.
fn rand_u32() -> u32 {
    use std::time::SystemTime;
    let d = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap();
    let pid = std::process::id() as u64;
    // Use nanos (high entropy) mixed with pid for cross-process uniqueness
    ((d.as_nanos() as u64 ^ pid.wrapping_mul(2654435761)) & 0xFFFF_FFFF) as u32
}

/// Minimal devx.toml with a long-running no-op service (no health check).
fn simple_toml(project_name: &str) -> String {
    format!(
        r#"[project]
name = "{project_name}"

[services.sleeper]
cmd = "sleep 300"
watch = false
"#,
    )
}

/// devx.toml with two services and a dependency.
fn two_service_toml(project_name: &str) -> String {
    format!(
        r#"[project]
name = "{project_name}"

[services.base]
cmd = "sleep 300"
watch = false

[services.dependent]
cmd = "sleep 300"
depends_on = ["base"]
watch = false
"#,
    )
}

/// devx.toml with an invalid config (missing cmd).
fn invalid_toml(project_name: &str) -> String {
    format!(
        r#"[project]
name = "{project_name}"

[services.broken]
dir = "."
"#,
    )
}

fn socket_path(project_name: &str) -> PathBuf {
    PathBuf::from(format!("/tmp/devx-{}.sock", project_name))
}

fn pid_path(project_name: &str) -> PathBuf {
    PathBuf::from(format!("/tmp/devx-{}.pid", project_name))
}

fn log_path(project_name: &str) -> PathBuf {
    PathBuf::from(format!("/tmp/devx-{}.log", project_name))
}

/// Wait until a predicate is true, with timeout.
fn wait_for(timeout: Duration, poll: Duration, pred: impl Fn() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if pred() {
            return true;
        }
        thread::sleep(poll);
    }
    false
}

/// Send a raw JSON command to the control socket and return the response.
fn send_socket_command(project_name: &str, cmd: &str) -> String {
    let path = socket_path(project_name);
    let mut stream = UnixStream::connect(&path).expect("failed to connect to socket");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    write!(stream, "{}\n", cmd).expect("failed to write to socket");
    stream
        .shutdown(std::net::Shutdown::Write)
        .expect("failed to shutdown write");
    let mut reader = BufReader::new(stream);
    let mut response = String::new();
    reader
        .read_line(&mut response)
        .expect("failed to read response");
    response
}

/// Clean up any leftover files from a test project (belt-and-suspenders).
fn cleanup_project(project_name: &str) {
    // Try to shut down via socket first
    if socket_path(project_name).exists() {
        let _ = send_socket_command(project_name, r#"{"cmd":"shutdown"}"#);
        thread::sleep(Duration::from_millis(500));
    }
    // Kill via PID if still around
    if let Ok(pid_str) = fs::read_to_string(pid_path(project_name)) {
        if let Ok(pid) = pid_str.trim().parse::<i32>() {
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
            thread::sleep(Duration::from_millis(200));
        }
    }
    let _ = fs::remove_file(socket_path(project_name));
    let _ = fs::remove_file(pid_path(project_name));
    let _ = fs::remove_file(log_path(project_name));
}

// ---------------------------------------------------------------------------
// CLI Smoke Tests
// ---------------------------------------------------------------------------

#[test]
fn cli_help_exits_zero() {
    let out = Command::new(devx_bin())
        .arg("--help")
        .output()
        .expect("failed to run devx");
    assert!(out.status.success(), "devx --help failed: {}", stderr(&out));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Local development orchestrator"));
}

#[test]
fn cli_up_help_shows_daemon_flag() {
    let out = Command::new(devx_bin())
        .args(["up", "--help"])
        .output()
        .expect("failed to run devx");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("--daemon"));
    assert!(stdout.contains("-d"));
}

#[test]
fn cli_logs_help_shows_flags() {
    let out = Command::new(devx_bin())
        .args(["logs", "--help"])
        .output()
        .expect("failed to run devx");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("--follow"));
    assert!(stdout.contains("--service"));
    assert!(stdout.contains("--lines"));
}

#[test]
fn cli_no_devx_toml_fails() {
    let dir = TempDir::new().unwrap();
    let out = devx(dir.path()).args(["check"]).output().unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("devx.toml not found"),
        "expected 'devx.toml not found', got: {}",
        stderr
    );
}

// ---------------------------------------------------------------------------
// Config Validation (via `devx check`)
// ---------------------------------------------------------------------------

#[test]
fn check_valid_config() {
    let name = unique_name("check-valid");
    let (dir, _) = setup_project(&simple_toml(&name));

    let out = devx(dir.path()).args(["check"]).output().unwrap();
    assert!(out.status.success(), "devx check failed: {}", stderr(&out));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("valid (1 services)"));
}

#[test]
fn check_two_services() {
    let name = unique_name("check-two");
    let (dir, _) = setup_project(&two_service_toml(&name));

    let out = devx(dir.path()).args(["check"]).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("valid (2 services)"));
}

#[test]
fn check_invalid_config_missing_cmd() {
    let name = unique_name("check-invalid");
    let (dir, _) = setup_project(&invalid_toml(&name));

    let out = devx(dir.path()).args(["check"]).output().unwrap();
    assert!(!out.status.success(), "devx check should fail for invalid config");
}

#[test]
fn check_invalid_config_bad_dependency() {
    let name = unique_name("check-baddep");
    let toml = format!(
        r#"[project]
name = "{name}"

[services.api]
cmd = "sleep 1"
depends_on = ["nonexistent"]
"#
    );
    let (dir, _) = setup_project(&toml);

    let out = devx(dir.path()).args(["check"]).output().unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("nonexistent"),
        "error should mention the missing dependency, got: {}",
        stderr
    );
}

// ---------------------------------------------------------------------------
// Status when not running
// ---------------------------------------------------------------------------

#[test]
fn status_when_not_running() {
    let name = unique_name("status-none");
    let (dir, _) = setup_project(&simple_toml(&name));

    let out = devx(dir.path()).args(["status"]).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("not running"),
        "expected 'not running', got: {}",
        stdout
    );
}

// ---------------------------------------------------------------------------
// Daemon Lifecycle (integration + E2E)
// ---------------------------------------------------------------------------

#[test]
fn daemon_start_creates_pid_and_socket() {
    let name = unique_name("daemon-start");
    let (dir, _) = setup_project(&simple_toml(&name));
    // Ensure clean slate
    cleanup_project(&name);

    let out = devx(dir.path()).args(["up", "-d"]).output().unwrap();
    assert!(
        out.status.success(),
        "devx up -d failed: {}",
        stderr(&out)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("daemon started"),
        "expected 'daemon started' in output, got: {}",
        stdout
    );

    // Wait for socket to appear (daemon needs a moment to start)
    let socket_ready = wait_for(Duration::from_secs(5), Duration::from_millis(100), || {
        socket_path(&name).exists()
    });
    assert!(socket_ready, "control socket didn't appear within 5s");

    // PID file should exist
    assert!(
        pid_path(&name).exists(),
        "PID file should exist after daemon start"
    );

    // Log file should exist
    assert!(
        log_path(&name).exists(),
        "log file should exist after daemon start"
    );

    // PID file should contain a valid number
    let pid_str = fs::read_to_string(pid_path(&name)).unwrap();
    let pid: u32 = pid_str.trim().parse().expect("PID file should contain a number");
    assert!(pid > 0);

    // Process should be alive
    let alive = unsafe { libc::kill(pid as i32, 0) == 0 };
    assert!(alive, "daemon process (pid {}) should be alive", pid);

    // Cleanup
    cleanup_project(&name);
}

#[test]
fn daemon_down_stops_and_cleans_up() {
    let name = unique_name("daemon-down");
    let (dir, _) = setup_project(&simple_toml(&name));
    cleanup_project(&name);

    // Start daemon
    let out = devx(dir.path()).args(["up", "-d"]).output().unwrap();
    assert!(out.status.success(), "start failed: {}", stderr(&out));

    let socket_ready = wait_for(Duration::from_secs(5), Duration::from_millis(100), || {
        socket_path(&name).exists()
    });
    assert!(socket_ready, "socket didn't appear");

    // Read PID before stopping
    let pid_str = fs::read_to_string(pid_path(&name)).unwrap();
    let pid: i32 = pid_str.trim().parse().unwrap();

    // Stop daemon
    let out = devx(dir.path()).args(["down"]).output().unwrap();
    assert!(out.status.success(), "devx down failed: {}", stderr(&out));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("devx stopped"));

    // Wait for process to actually exit
    let exited = wait_for(Duration::from_secs(5), Duration::from_millis(100), || {
        unsafe { libc::kill(pid, 0) != 0 }
    });
    assert!(exited, "daemon process should have exited");

    // PID file should be cleaned up
    assert!(
        !pid_path(&name).exists(),
        "PID file should be removed after down"
    );

    // Socket should be cleaned up
    assert!(
        !socket_path(&name).exists(),
        "socket should be removed after down"
    );
}

#[test]
fn daemon_prevents_double_start() {
    let name = unique_name("daemon-double");
    let (dir, _) = setup_project(&simple_toml(&name));
    cleanup_project(&name);

    // Start first instance
    let out = devx(dir.path()).args(["up", "-d"]).output().unwrap();
    assert!(out.status.success(), "first start failed: {}", stderr(&out));

    let socket_ready = wait_for(Duration::from_secs(5), Duration::from_millis(100), || {
        socket_path(&name).exists()
    });
    assert!(socket_ready, "socket didn't appear");

    // Try to start a second instance
    let out = devx(dir.path()).args(["up", "-d"]).output().unwrap();
    assert!(
        !out.status.success(),
        "second devx up -d should fail when already running"
    );
    let stderr_str = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr_str.contains("already running"),
        "expected 'already running' error, got: {}",
        stderr_str
    );

    cleanup_project(&name);
}

// ---------------------------------------------------------------------------
// Control Socket Protocol
// ---------------------------------------------------------------------------

#[test]
fn control_socket_shutdown() {
    let name = unique_name("sock-shutdown");
    let (dir, _) = setup_project(&simple_toml(&name));
    cleanup_project(&name);

    devx(dir.path()).args(["up", "-d"]).output().unwrap();
    wait_for(Duration::from_secs(5), Duration::from_millis(100), || {
        socket_path(&name).exists()
    });

    let resp = send_socket_command(&name, r#"{"cmd":"shutdown"}"#);
    assert!(
        resp.contains("\"ok\""),
        "shutdown should return ok, got: {}",
        resp
    );

    // Wait for cleanup
    wait_for(Duration::from_secs(5), Duration::from_millis(100), || {
        !socket_path(&name).exists()
    });
}

#[test]
fn control_socket_status() {
    let name = unique_name("sock-status");
    let (dir, _) = setup_project(&simple_toml(&name));
    cleanup_project(&name);

    devx(dir.path()).args(["up", "-d"]).output().unwrap();
    let ready = wait_for(Duration::from_secs(5), Duration::from_millis(100), || {
        socket_path(&name).exists()
    });
    assert!(ready, "socket didn't appear");

    // Give orchestrator a moment to spawn services
    thread::sleep(Duration::from_secs(1));

    let resp = send_socket_command(&name, r#"{"cmd":"status"}"#);
    let json: serde_json::Value = serde_json::from_str(resp.trim()).expect("status should be JSON");

    assert_eq!(json["project"], name);
    let services = json["services"].as_array().expect("services should be an array");
    assert_eq!(services.len(), 1);
    assert_eq!(services[0]["name"], "sleeper");

    cleanup_project(&name);
}

#[test]
fn control_socket_unknown_command() {
    let name = unique_name("sock-unknown");
    let (dir, _) = setup_project(&simple_toml(&name));
    cleanup_project(&name);

    devx(dir.path()).args(["up", "-d"]).output().unwrap();
    wait_for(Duration::from_secs(5), Duration::from_millis(100), || {
        socket_path(&name).exists()
    });

    let resp = send_socket_command(&name, r#"{"cmd":"bogus"}"#);
    assert!(
        resp.contains("unknown command"),
        "expected 'unknown command', got: {}",
        resp
    );

    cleanup_project(&name);
}

#[test]
fn control_socket_invalid_json() {
    let name = unique_name("sock-badjson");
    let (dir, _) = setup_project(&simple_toml(&name));
    cleanup_project(&name);

    devx(dir.path()).args(["up", "-d"]).output().unwrap();
    wait_for(Duration::from_secs(5), Duration::from_millis(100), || {
        socket_path(&name).exists()
    });

    let resp = send_socket_command(&name, "not json at all");
    assert!(
        resp.contains("invalid json"),
        "expected 'invalid json', got: {}",
        resp
    );

    cleanup_project(&name);
}

// ---------------------------------------------------------------------------
// devx status (CLI, while daemon running)
// ---------------------------------------------------------------------------

#[test]
fn status_shows_services_while_running() {
    let name = unique_name("status-live");
    let (dir, _) = setup_project(&two_service_toml(&name));
    cleanup_project(&name);

    devx(dir.path()).args(["up", "-d"]).output().unwrap();
    let ready = wait_for(Duration::from_secs(5), Duration::from_millis(100), || {
        socket_path(&name).exists()
    });
    assert!(ready, "socket didn't appear");
    thread::sleep(Duration::from_secs(1));

    let out = devx(dir.path()).args(["status"]).output().unwrap();
    assert!(out.status.success(), "devx status failed: {}", stderr(&out));

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("SERVICE"), "should have table header");
    assert!(stdout.contains("base"), "should list 'base' service");
    assert!(stdout.contains("dependent"), "should list 'dependent' service");

    cleanup_project(&name);
}

// ---------------------------------------------------------------------------
// devx logs
// ---------------------------------------------------------------------------

#[test]
fn logs_shows_daemon_output() {
    let name = unique_name("logs-basic");
    let (dir, _) = setup_project(&simple_toml(&name));
    cleanup_project(&name);

    devx(dir.path()).args(["up", "-d"]).output().unwrap();
    let ready = wait_for(Duration::from_secs(5), Duration::from_millis(100), || {
        socket_path(&name).exists()
    });
    assert!(ready, "socket didn't appear");

    // Wait for some log entries to accumulate
    thread::sleep(Duration::from_secs(2));

    let out = devx(dir.path()).args(["logs"]).output().unwrap();
    assert!(out.status.success(), "devx logs failed: {}", stderr(&out));

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("[devx]"),
        "logs should contain devx system messages, got: {}",
        stdout
    );
    assert!(
        stdout.contains("daemon started"),
        "logs should contain 'daemon started', got: {}",
        stdout
    );

    cleanup_project(&name);
}

#[test]
fn logs_respects_line_count() {
    let name = unique_name("logs-lines");
    let (dir, _) = setup_project(&simple_toml(&name));
    cleanup_project(&name);

    devx(dir.path()).args(["up", "-d"]).output().unwrap();
    wait_for(Duration::from_secs(5), Duration::from_millis(100), || {
        socket_path(&name).exists()
    });
    thread::sleep(Duration::from_secs(2));

    // Request only 2 lines
    let out = devx(dir.path())
        .args(["logs", "-n", "2"])
        .output()
        .unwrap();
    assert!(out.status.success());

    let stdout = String::from_utf8_lossy(&out.stdout);
    let line_count = stdout.trim().lines().count();
    assert!(
        line_count <= 2,
        "requested 2 lines but got {}: {}",
        line_count,
        stdout
    );

    cleanup_project(&name);
}

#[test]
fn logs_no_daemon_no_logfile_fails() {
    let name = unique_name("logs-nofile");
    let (dir, _) = setup_project(&simple_toml(&name));
    // Make sure there's no log file
    let _ = fs::remove_file(log_path(&name));

    let out = devx(dir.path()).args(["logs"]).output().unwrap();
    assert!(!out.status.success());
    let stderr_str = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr_str.contains("no log file"),
        "expected 'no log file' error, got: {}",
        stderr_str
    );
}

// ---------------------------------------------------------------------------
// E2E: Full lifecycle
// ---------------------------------------------------------------------------

#[test]
fn e2e_full_daemon_lifecycle() {
    let name = unique_name("e2e-lifecycle");
    let (dir, _) = setup_project(&two_service_toml(&name));
    cleanup_project(&name);

    // 1. Start daemon
    let out = devx(dir.path()).args(["up", "-d"]).output().unwrap();
    assert!(out.status.success(), "up -d failed: {}", stderr(&out));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("daemon started"));

    // 2. Wait for socket
    let ready = wait_for(Duration::from_secs(5), Duration::from_millis(100), || {
        socket_path(&name).exists()
    });
    assert!(ready, "socket didn't appear");
    thread::sleep(Duration::from_secs(2));

    // 3. Check status — should show both services
    let out = devx(dir.path()).args(["status"]).output().unwrap();
    assert!(out.status.success(), "status failed: {}", stderr(&out));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("base"));
    assert!(stdout.contains("dependent"));

    // 4. Check logs — should have startup entries
    let out = devx(dir.path()).args(["logs"]).output().unwrap();
    assert!(out.status.success(), "logs failed: {}", stderr(&out));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("[devx]"));

    // 5. Restart a service via CLI
    let out = devx(dir.path())
        .args(["restart", "base"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "restart failed: {}",
        stderr(&out)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("restarted"));

    // 6. Status should still work after restart
    thread::sleep(Duration::from_secs(1));
    let out = devx(dir.path()).args(["status"]).output().unwrap();
    assert!(out.status.success());

    // 7. Stop daemon
    let out = devx(dir.path()).args(["down"]).output().unwrap();
    assert!(out.status.success(), "down failed: {}", stderr(&out));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("devx stopped"));

    // 8. Verify everything cleaned up
    let cleaned = wait_for(Duration::from_secs(5), Duration::from_millis(100), || {
        !pid_path(&name).exists() && !socket_path(&name).exists()
    });
    assert!(cleaned, "PID and socket should be cleaned up");

    // 9. Status should say not running
    let out = devx(dir.path()).args(["status"]).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("not running"));

    // 10. Logs should still be readable (post-mortem)
    let out = devx(dir.path()).args(["logs"]).output().unwrap();
    assert!(out.status.success(), "post-mortem logs should work");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("daemon stopped"));

    // Final cleanup
    let _ = fs::remove_file(log_path(&name));
}

#[test]
fn e2e_service_with_output() {
    let name = unique_name("e2e-output");
    let toml = format!(
        r#"[project]
name = "{name}"

[services.echo]
cmd = "sh -c 'echo hello-from-devx && sleep 300'"
watch = false
"#
    );
    let (dir, _) = setup_project(&toml);
    cleanup_project(&name);

    devx(dir.path()).args(["up", "-d"]).output().unwrap();
    wait_for(Duration::from_secs(5), Duration::from_millis(100), || {
        socket_path(&name).exists()
    });

    // Wait for the echo output to land in the log
    thread::sleep(Duration::from_secs(2));

    let out = devx(dir.path()).args(["logs"]).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("hello-from-devx"),
        "logs should capture service stdout, got: {}",
        stdout
    );

    cleanup_project(&name);
}

#[test]
fn e2e_service_filter_in_logs() {
    let name = unique_name("e2e-filter");
    let toml = format!(
        r#"[project]
name = "{name}"

[services.alpha]
cmd = "sh -c 'echo alpha-output && sleep 300'"
watch = false

[services.beta]
cmd = "sh -c 'echo beta-output && sleep 300'"
watch = false
"#
    );
    let (dir, _) = setup_project(&toml);
    cleanup_project(&name);

    devx(dir.path()).args(["up", "-d"]).output().unwrap();
    wait_for(Duration::from_secs(5), Duration::from_millis(100), || {
        socket_path(&name).exists()
    });
    thread::sleep(Duration::from_secs(2));

    // Filter for alpha only
    let out = devx(dir.path())
        .args(["logs", "-s", "alpha"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("alpha"),
        "filtered logs should contain alpha, got: {}",
        stdout
    );
    // Beta lines should NOT appear (they don't contain "[alpha]")
    assert!(
        !stdout.contains("[beta]"),
        "filtered logs should not contain [beta], got: {}",
        stdout
    );

    cleanup_project(&name);
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

#[test]
fn down_when_not_running_fails() {
    let name = unique_name("down-norun");
    let (dir, _) = setup_project(&simple_toml(&name));
    cleanup_project(&name);

    let out = devx(dir.path()).args(["down"]).output().unwrap();
    assert!(
        !out.status.success(),
        "devx down should fail when not running"
    );
    let stderr_str = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr_str.contains("no running devx instance")
            || stderr_str.contains("not found"),
        "expected socket-not-found error, got: {}",
        stderr_str
    );
}

#[test]
fn restart_when_not_running_fails() {
    let name = unique_name("restart-norun");
    let (dir, _) = setup_project(&simple_toml(&name));
    cleanup_project(&name);

    let out = devx(dir.path())
        .args(["restart", "sleeper"])
        .output()
        .unwrap();
    assert!(!out.status.success());
}

// ---------------------------------------------------------------------------
// Utility
// ---------------------------------------------------------------------------

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}
