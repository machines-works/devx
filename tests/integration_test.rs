//! Integration tests for the devx CLI.
//!
//! These tests invoke the real `devx` binary against real config files,
//! real sockets, and real processes. No mocks.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};
use std::{fs, thread};

use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

static TEST_COUNTER: AtomicU32 = AtomicU32::new(0);

fn devx_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_devx"))
}

fn devx(dir: &Path) -> Command {
    let mut cmd = Command::new(devx_bin());
    cmd.current_dir(dir);
    cmd
}

/// RAII guard that cleans up a test project's daemon on drop (even on panic).
struct TestProject {
    dir: TempDir,
    name: String,
}

impl TestProject {
    fn new(toml_content: &str) -> Self {
        let dir = TempDir::new().expect("failed to create temp dir");
        fs::write(dir.path().join("devx.toml"), toml_content).expect("failed to write devx.toml");
        let name = toml_content
            .lines()
            .find(|l| l.starts_with("name"))
            .and_then(|l| l.split('"').nth(1))
            .expect("devx.toml must have a project name")
            .to_string();
        cleanup_project(&name);
        Self { dir, name }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn devx(&self) -> Command {
        devx(self.path())
    }

    fn wait_for_socket(&self) {
        let ready = wait_for(Duration::from_secs(5), Duration::from_millis(100), || {
            socket_path(&self.name).exists()
        });
        assert!(ready, "control socket didn't appear within 5s");
    }
}

impl Drop for TestProject {
    fn drop(&mut self) {
        cleanup_project(&self.name);
    }
}

fn unique_name(base: &str) -> String {
    let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    format!("{}-{}-{}", base, pid, id)
}

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

fn cleanup_project(project_name: &str) {
    if socket_path(project_name).exists() {
        let _ = send_socket_command(project_name, r#"{"cmd":"shutdown"}"#);
        thread::sleep(Duration::from_millis(500));
    }
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

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
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
    assert!(stderr.contains("devx.toml not found"), "got: {}", stderr);
}

// ---------------------------------------------------------------------------
// Config Validation (via `devx check`)
// ---------------------------------------------------------------------------

#[test]
fn check_valid_config() {
    let p = TestProject::new(&simple_toml(&unique_name("check-valid")));
    let out = p.devx().args(["check"]).output().unwrap();
    assert!(out.status.success(), "devx check failed: {}", stderr(&out));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("valid (1 services)"));
}

#[test]
fn check_two_services() {
    let p = TestProject::new(&two_service_toml(&unique_name("check-two")));
    let out = p.devx().args(["check"]).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("valid (2 services)"));
}

#[test]
fn check_invalid_config_missing_cmd() {
    let p = TestProject::new(&invalid_toml(&unique_name("check-invalid")));
    let out = p.devx().args(["check"]).output().unwrap();
    assert!(
        !out.status.success(),
        "devx check should fail for invalid config"
    );
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
    let p = TestProject::new(&toml);
    let out = p.devx().args(["check"]).output().unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("nonexistent"), "got: {}", stderr);
}

// ---------------------------------------------------------------------------
// Status when not running
// ---------------------------------------------------------------------------

#[test]
fn status_when_not_running() {
    let p = TestProject::new(&simple_toml(&unique_name("status-none")));
    let out = p.devx().args(["status"]).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("not running"), "got: {}", stdout);
}

// ---------------------------------------------------------------------------
// Daemon Lifecycle (integration + E2E)
// ---------------------------------------------------------------------------

#[test]
fn daemon_start_creates_pid_and_socket() {
    let p = TestProject::new(&simple_toml(&unique_name("daemon-start")));

    let out = p.devx().args(["up", "-d"]).output().unwrap();
    assert!(out.status.success(), "devx up -d failed: {}", stderr(&out));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("daemon started"), "got: {}", stdout);

    p.wait_for_socket();

    assert!(pid_path(&p.name).exists(), "PID file should exist");
    assert!(log_path(&p.name).exists(), "log file should exist");

    let pid_str = fs::read_to_string(pid_path(&p.name)).unwrap();
    let pid: u32 = pid_str
        .trim()
        .parse()
        .expect("PID file should contain a number");
    assert!(pid > 0);
    assert!(
        unsafe { libc::kill(pid as i32, 0) == 0 },
        "daemon should be alive"
    );
}

#[test]
fn daemon_down_stops_and_cleans_up() {
    let p = TestProject::new(&simple_toml(&unique_name("daemon-down")));

    let out = p.devx().args(["up", "-d"]).output().unwrap();
    assert!(out.status.success(), "start failed: {}", stderr(&out));
    p.wait_for_socket();

    let pid_str = fs::read_to_string(pid_path(&p.name)).unwrap();
    let pid: i32 = pid_str.trim().parse().unwrap();

    let out = p.devx().args(["down"]).output().unwrap();
    assert!(out.status.success(), "devx down failed: {}", stderr(&out));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("devx stopped"));

    let exited = wait_for(
        Duration::from_secs(5),
        Duration::from_millis(100),
        || unsafe { libc::kill(pid, 0) != 0 },
    );
    assert!(exited, "daemon process should have exited");
    assert!(!pid_path(&p.name).exists(), "PID file should be removed");
    assert!(!socket_path(&p.name).exists(), "socket should be removed");
}

#[test]
fn daemon_prevents_double_start() {
    let p = TestProject::new(&simple_toml(&unique_name("daemon-double")));

    let out = p.devx().args(["up", "-d"]).output().unwrap();
    assert!(out.status.success(), "first start failed: {}", stderr(&out));
    p.wait_for_socket();

    let out = p.devx().args(["up", "-d"]).output().unwrap();
    assert!(!out.status.success(), "second start should fail");
    let stderr_str = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr_str.contains("already running"),
        "got: {}",
        stderr_str
    );
}

// ---------------------------------------------------------------------------
// Control Socket Protocol
// ---------------------------------------------------------------------------

#[test]
fn control_socket_shutdown() {
    let p = TestProject::new(&simple_toml(&unique_name("sock-shutdown")));
    p.devx().args(["up", "-d"]).output().unwrap();
    p.wait_for_socket();

    let resp = send_socket_command(&p.name, r#"{"cmd":"shutdown"}"#);
    assert!(resp.contains("\"ok\""), "got: {}", resp);

    wait_for(Duration::from_secs(5), Duration::from_millis(100), || {
        !socket_path(&p.name).exists()
    });
}

#[test]
fn control_socket_status() {
    let p = TestProject::new(&simple_toml(&unique_name("sock-status")));
    p.devx().args(["up", "-d"]).output().unwrap();
    p.wait_for_socket();
    thread::sleep(Duration::from_secs(1));

    let resp = send_socket_command(&p.name, r#"{"cmd":"status"}"#);
    let json: serde_json::Value = serde_json::from_str(resp.trim()).expect("status should be JSON");

    assert_eq!(json["project"], p.name);
    let services = json["services"]
        .as_array()
        .expect("services should be array");
    assert_eq!(services.len(), 1);
    assert_eq!(services[0]["name"], "sleeper");
}

#[test]
fn control_socket_unknown_command() {
    let p = TestProject::new(&simple_toml(&unique_name("sock-unknown")));
    p.devx().args(["up", "-d"]).output().unwrap();
    p.wait_for_socket();

    let resp = send_socket_command(&p.name, r#"{"cmd":"bogus"}"#);
    assert!(resp.contains("unknown command"), "got: {}", resp);
}

#[test]
fn control_socket_invalid_json() {
    let p = TestProject::new(&simple_toml(&unique_name("sock-badjson")));
    p.devx().args(["up", "-d"]).output().unwrap();
    p.wait_for_socket();

    let resp = send_socket_command(&p.name, "not json at all");
    assert!(resp.contains("invalid json"), "got: {}", resp);
}

// ---------------------------------------------------------------------------
// devx status (CLI, while daemon running)
// ---------------------------------------------------------------------------

#[test]
fn status_shows_services_while_running() {
    let p = TestProject::new(&two_service_toml(&unique_name("status-live")));
    p.devx().args(["up", "-d"]).output().unwrap();
    p.wait_for_socket();
    thread::sleep(Duration::from_secs(1));

    let out = p.devx().args(["status"]).output().unwrap();
    assert!(out.status.success(), "devx status failed: {}", stderr(&out));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("SERVICE"), "should have table header");
    assert!(stdout.contains("base"), "should list 'base' service");
    assert!(
        stdout.contains("dependent"),
        "should list 'dependent' service"
    );
}

// ---------------------------------------------------------------------------
// devx logs
// ---------------------------------------------------------------------------

#[test]
fn logs_shows_daemon_output() {
    let p = TestProject::new(&simple_toml(&unique_name("logs-basic")));
    p.devx().args(["up", "-d"]).output().unwrap();
    p.wait_for_socket();
    thread::sleep(Duration::from_secs(2));

    let out = p.devx().args(["logs"]).output().unwrap();
    assert!(out.status.success(), "devx logs failed: {}", stderr(&out));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("[devx]"), "got: {}", stdout);
    assert!(stdout.contains("daemon started"), "got: {}", stdout);
}

#[test]
fn logs_respects_line_count() {
    let p = TestProject::new(&simple_toml(&unique_name("logs-lines")));
    p.devx().args(["up", "-d"]).output().unwrap();
    p.wait_for_socket();
    thread::sleep(Duration::from_secs(2));

    let out = p.devx().args(["logs", "-n", "2"]).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let line_count = stdout.trim().lines().count();
    assert!(
        line_count <= 2,
        "requested 2 lines but got {}: {}",
        line_count,
        stdout
    );
}

#[test]
fn logs_no_daemon_no_logfile_fails() {
    let p = TestProject::new(&simple_toml(&unique_name("logs-nofile")));
    let _ = fs::remove_file(log_path(&p.name));

    let out = p.devx().args(["logs"]).output().unwrap();
    assert!(!out.status.success());
    let stderr_str = String::from_utf8_lossy(&out.stderr);
    assert!(stderr_str.contains("no log file"), "got: {}", stderr_str);
}

// ---------------------------------------------------------------------------
// E2E: Full lifecycle
// ---------------------------------------------------------------------------

#[test]
fn e2e_full_daemon_lifecycle() {
    let p = TestProject::new(&two_service_toml(&unique_name("e2e-lifecycle")));

    // 1. Start daemon
    let out = p.devx().args(["up", "-d"]).output().unwrap();
    assert!(out.status.success(), "up -d failed: {}", stderr(&out));
    assert!(String::from_utf8_lossy(&out.stdout).contains("daemon started"));

    // 2. Wait for socket
    p.wait_for_socket();
    thread::sleep(Duration::from_secs(2));

    // 3. Status — both services
    let out = p.devx().args(["status"]).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("base"));
    assert!(stdout.contains("dependent"));

    // 4. Logs — startup entries
    let out = p.devx().args(["logs"]).output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("[devx]"));

    // 5. Restart a service
    let out = p.devx().args(["restart", "base"]).output().unwrap();
    assert!(out.status.success(), "restart failed: {}", stderr(&out));
    assert!(String::from_utf8_lossy(&out.stdout).contains("restarted"));

    // 6. Status still works after restart
    thread::sleep(Duration::from_secs(1));
    assert!(p.devx().args(["status"]).output().unwrap().status.success());

    // 7. Stop daemon
    let out = p.devx().args(["down"]).output().unwrap();
    assert!(out.status.success(), "down failed: {}", stderr(&out));
    assert!(String::from_utf8_lossy(&out.stdout).contains("devx stopped"));

    // 8. Everything cleaned up
    let cleaned = wait_for(Duration::from_secs(5), Duration::from_millis(100), || {
        !pid_path(&p.name).exists() && !socket_path(&p.name).exists()
    });
    assert!(cleaned, "PID and socket should be cleaned up");

    // 9. Status says not running
    let out = p.devx().args(["status"]).output().unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("not running"));

    // 10. Post-mortem logs
    let out = p.devx().args(["logs"]).output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("daemon stopped"));
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
    let p = TestProject::new(&toml);
    p.devx().args(["up", "-d"]).output().unwrap();
    p.wait_for_socket();
    thread::sleep(Duration::from_secs(2));

    let out = p.devx().args(["logs"]).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("hello-from-devx"), "got: {}", stdout);
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
    let p = TestProject::new(&toml);
    p.devx().args(["up", "-d"]).output().unwrap();
    p.wait_for_socket();
    thread::sleep(Duration::from_secs(2));

    let out = p.devx().args(["logs", "-s", "alpha"]).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("alpha"), "got: {}", stdout);
    assert!(!stdout.contains("[beta]"), "got: {}", stdout);
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

#[test]
fn down_when_not_running_fails() {
    let p = TestProject::new(&simple_toml(&unique_name("down-norun")));
    let out = p.devx().args(["down"]).output().unwrap();
    assert!(
        !out.status.success(),
        "devx down should fail when not running"
    );
    let stderr_str = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr_str.contains("no running devx instance") || stderr_str.contains("not found"),
        "got: {}",
        stderr_str
    );
}

#[test]
fn restart_when_not_running_fails() {
    let p = TestProject::new(&simple_toml(&unique_name("restart-norun")));
    let out = p.devx().args(["restart", "sleeper"]).output().unwrap();
    assert!(!out.status.success());
}
