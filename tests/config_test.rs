use devx::config::{interpolate, DevxConfig};
use std::collections::HashMap;

#[test]
fn test_parse_minimal_config() {
    let toml = r#"
[project]
name = "myapp"

[services.web]
cmd = "npm start"
"#;
    let cfg = DevxConfig::parse(toml).unwrap();
    assert_eq!(cfg.project.name, "myapp");
    assert!(cfg.infra.is_none());
    let web = cfg.services.get("web").unwrap();
    assert_eq!(web.cmd, "npm start");
    assert!(web.dir.is_none());
    assert!(web.port.is_none());
    assert!(web.health.is_none());
    assert!(web.env.is_empty());
    assert!(web.depends_on.is_empty());
}

#[test]
fn test_parse_full_config() {
    let toml = r#"
[project]
name = "fullapp"

[infra]
compose = "docker-compose.yml"

[services.db]
cmd = "postgres"
port = 5432

[services.api]
cmd = "go run ./cmd/api"
dir = "./api"
port = 8080
health = "http://localhost:8080/health"
depends_on = ["db"]

[services.api.env]
DATABASE_URL = "postgres://localhost:5432/mydb"
DEBUG = "true"
"#;
    let cfg = DevxConfig::parse(toml).unwrap();
    assert_eq!(cfg.project.name, "fullapp");
    assert_eq!(cfg.infra.unwrap().compose, "docker-compose.yml");

    let db = cfg.services.get("db").unwrap();
    assert_eq!(db.port, Some(5432));

    let api = cfg.services.get("api").unwrap();
    assert_eq!(api.cmd, "go run ./cmd/api");
    assert_eq!(api.dir.as_deref(), Some("./api"));
    assert_eq!(api.health.as_deref(), Some("http://localhost:8080/health"));
    assert_eq!(api.depends_on, vec!["db"]);
    assert_eq!(api.env.get("DEBUG").map(String::as_str), Some("true"));
}

#[test]
fn test_interpolate_port() {
    let mut actual = HashMap::new();
    actual.insert("api".to_string(), 8080u16);
    let proxy: HashMap<String, u16> = HashMap::new();
    let result = interpolate("http://localhost:${port}/health", "api", &actual, &proxy);
    assert_eq!(result, "http://localhost:8080/health");
}

#[test]
fn test_interpolate_proxy_ref() {
    let actual: HashMap<String, u16> = HashMap::new();
    let mut proxy = HashMap::new();
    proxy.insert("api".to_string(), 9080u16);
    let result = interpolate("PROXY=${proxy:api}", "web", &actual, &proxy);
    assert_eq!(result, "PROXY=9080");
}

#[test]
fn test_diff_unchanged() {
    let toml = r#"
[project]
name = "app"

[services.api]
cmd = "go run ."
port = 8080
"#;
    let a = DevxConfig::parse(toml).unwrap();
    let b = DevxConfig::parse(toml).unwrap();
    let diff = a.diff(&b);
    assert!(diff.added.is_empty());
    assert!(diff.removed.is_empty());
    assert!(diff.changed.is_empty());
    assert_eq!(diff.unchanged, vec!["api"]);
}

#[test]
fn test_diff_added_removed_changed() {
    let old_toml = r#"
[project]
name = "app"

[services.api]
cmd = "go run ."
port = 8080

[services.db]
cmd = "postgres"
"#;
    let new_toml = r#"
[project]
name = "app"

[services.api]
cmd = "go run ./cmd/api"
port = 8080

[services.worker]
cmd = "python worker.py"
"#;
    let old = DevxConfig::parse(old_toml).unwrap();
    let new = DevxConfig::parse(new_toml).unwrap();
    let mut diff = old.diff(&new);

    diff.added.sort();
    diff.removed.sort();
    diff.changed.sort();
    diff.unchanged.sort();

    assert_eq!(diff.added, vec!["worker"]);
    assert_eq!(diff.removed, vec!["db"]);
    assert_eq!(diff.changed, vec!["api"]);
    assert!(diff.unchanged.is_empty());
}

#[test]
fn test_missing_cmd_is_error() {
    // TOML requires cmd field to be present for deserialization
    // An empty string should also fail validation
    let toml = r#"
[project]
name = "broken"

[services.bad]
cmd = ""
"#;
    let err = DevxConfig::parse(toml);
    assert!(err.is_err(), "expected error for empty cmd");
}
