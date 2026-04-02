use devx::detect::Framework;
use std::path::Path;

#[test]
fn test_detect_vite() {
    let f = Framework::detect("npm run dev", Path::new("/nonexistent"));
    // Without package.json, falls back to Unknown
    assert!(matches!(f, Framework::Unknown));
}

#[test]
fn test_detect_vite_from_cmd() {
    let f = Framework::detect("vite dev", Path::new("/nonexistent"));
    assert!(matches!(f, Framework::Vite));
}

#[test]
fn test_detect_uvicorn() {
    let f = Framework::detect("uvicorn app:app", Path::new("/nonexistent"));
    assert!(matches!(f, Framework::Python));
}

#[test]
fn test_detect_go() {
    let f = Framework::detect("go run ./cmd/api", Path::new("/nonexistent"));
    assert!(matches!(f, Framework::Go));
}

#[test]
fn test_inject_vite_port() {
    let result = Framework::Vite.inject_port_flag("npm run dev", 3000);
    assert_eq!(result, Some("npm run dev --port 3000 --host".to_string()));
}

#[test]
fn test_no_inject_when_port_present() {
    let result = Framework::Vite.inject_port_flag("npm run dev --port ${port}", 3000);
    assert_eq!(result, None);
}

#[test]
fn test_inject_uvicorn_port() {
    let result = Framework::Python.inject_port_flag("uvicorn app:app --host 0.0.0.0", 8090);
    assert_eq!(
        result,
        Some("uvicorn app:app --host 0.0.0.0 --port 8090".to_string())
    );
}

#[test]
fn test_detect_encore() {
    let f = Framework::detect("encore run", Path::new("/nonexistent"));
    assert!(matches!(f, Framework::Encore));
}

#[test]
fn test_encore_injects_port_flag() {
    let result = Framework::Encore.inject_port_flag("encore run", 5000);
    assert_eq!(result, Some("encore run --port=5000".to_string()));
}

#[test]
fn test_encore_no_inject_when_port_present() {
    let result = Framework::Encore.inject_port_flag("encore run --port=4000", 5000);
    assert_eq!(result, None);
}

#[test]
fn test_encore_no_port_env() {
    assert!(!Framework::Encore.injects_port_env());
}
