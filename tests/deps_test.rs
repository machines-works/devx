use devx::deps::{resolve_order, resolve_with_filter};
use std::collections::HashMap;

fn graph(pairs: &[(&str, &[&str])]) -> HashMap<String, Vec<String>> {
    pairs
        .iter()
        .map(|(k, vs)| (k.to_string(), vs.iter().map(|s| s.to_string()).collect()))
        .collect()
}

#[test]
fn test_no_dependencies() {
    let g = graph(&[("a", &[]), ("b", &[]), ("c", &[])]);
    let waves = resolve_order(&g).unwrap();
    assert_eq!(waves.len(), 1);
    let mut wave = waves[0].clone();
    wave.sort();
    assert_eq!(wave, vec!["a", "b", "c"]);
}

#[test]
fn test_linear_chain() {
    // a depends on b, b depends on c → start order: c, b, a
    let g = graph(&[("a", &["b"]), ("b", &["c"]), ("c", &[])]);
    let waves = resolve_order(&g).unwrap();
    assert_eq!(waves.len(), 3);
    assert_eq!(waves[0], vec!["c"]);
    assert_eq!(waves[1], vec!["b"]);
    assert_eq!(waves[2], vec!["a"]);
}

#[test]
fn test_diamond_dependency() {
    // web depends on api and worker, both depend on db
    let g = graph(&[
        ("db", &[]),
        ("api", &["db"]),
        ("worker", &["db"]),
        ("web", &["api", "worker"]),
    ]);
    let waves = resolve_order(&g).unwrap();
    assert_eq!(waves.len(), 3);
    assert_eq!(waves[0], vec!["db"]);
    let mut w1 = waves[1].clone();
    w1.sort();
    assert_eq!(w1, vec!["api", "worker"]);
    assert_eq!(waves[2], vec!["web"]);
}

#[test]
fn test_cycle_detected() {
    let g = graph(&[("a", &["b"]), ("b", &["a"])]);
    let result = resolve_order(&g);
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("cycle"));
}

#[test]
fn test_filter_services_includes_transitive_deps() {
    // web → api → db
    let g = graph(&[
        ("db", &[]),
        ("api", &["db"]),
        ("web", &["api"]),
        ("standalone", &[]),
    ]);
    let requested = vec!["web".to_string()];
    let waves = resolve_with_filter(&g, &requested).unwrap();

    // Flatten and sort to check membership (wave ordering still valid)
    let all: Vec<String> = waves.into_iter().flatten().collect();
    assert!(all.contains(&"web".to_string()));
    assert!(all.contains(&"api".to_string()));
    assert!(all.contains(&"db".to_string()));
    assert!(!all.contains(&"standalone".to_string()));
}
