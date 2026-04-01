use anyhow::{bail, Result};
use std::collections::{HashMap, HashSet, VecDeque};

/// Kahn's topological sort returning waves (parallel batches).
/// Each wave contains services that can start simultaneously.
/// Nodes within a wave are sorted alphabetically for deterministic output.
pub fn resolve_order(graph: &HashMap<String, Vec<String>>) -> Result<Vec<Vec<String>>> {
    // Build adjacency list (dependency → dependents)
    let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();

    for (node, deps) in graph {
        dependents.entry(node.as_str()).or_default();
        for dep in deps {
            dependents
                .entry(dep.as_str())
                .or_default()
                .push(node.as_str());
        }
    }

    // Count edges for each node (in-degree = number of deps)
    let mut degree: HashMap<&str, usize> = HashMap::new();
    for (node, deps) in graph {
        *degree.entry(node.as_str()).or_insert(0) += deps.len();
        for dep in deps {
            degree.entry(dep.as_str()).or_insert(0);
        }
    }

    let total = degree.len();
    let mut queue: VecDeque<&str> = degree
        .iter()
        .filter(|&(_, &d)| d == 0)
        .map(|(&n, _)| n)
        .collect();

    let mut waves: Vec<Vec<String>> = Vec::new();
    let mut processed = 0usize;

    while !queue.is_empty() {
        // Collect current wave (all nodes with in-degree 0)
        let mut wave: Vec<&str> = queue.drain(..).collect();
        wave.sort_unstable();
        processed += wave.len();

        // Reduce in-degree for dependents and collect next wave
        let mut next: Vec<&str> = Vec::new();
        for &node in &wave {
            if let Some(deps) = dependents.get(node) {
                for &dep in deps {
                    let d = degree.get_mut(dep).unwrap();
                    *d -= 1;
                    if *d == 0 {
                        next.push(dep);
                    }
                }
            }
        }

        waves.push(wave.into_iter().map(str::to_owned).collect());
        queue.extend(next);
    }

    if processed != total {
        bail!("dependency graph contains a cycle");
    }

    Ok(waves)
}

/// BFS to collect transitive deps of `requested`, then topological sort the subgraph.
pub fn resolve_with_filter(
    graph: &HashMap<String, Vec<String>>,
    requested: &[String],
) -> Result<Vec<Vec<String>>> {
    let mut needed: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<&str> = requested.iter().map(String::as_str).collect();

    while let Some(node) = queue.pop_front() {
        if needed.insert(node.to_owned())
            && let Some(deps) = graph.get(node)
        {
            for dep in deps {
                if !needed.contains(dep.as_str()) {
                    queue.push_back(dep.as_str());
                }
            }
        }
    }

    // Build subgraph restricted to `needed`
    let subgraph: HashMap<String, Vec<String>> = needed
        .iter()
        .map(|n| {
            let deps = graph
                .get(n)
                .map(|ds| ds.iter().filter(|d| needed.contains(*d)).cloned().collect())
                .unwrap_or_default();
            (n.clone(), deps)
        })
        .collect();

    resolve_order(&subgraph)
}
