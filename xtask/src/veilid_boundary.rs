//! The Veilid boundary is transitive linkage (ADR 0014, plan C8).
//!
//! A crate links veilid-core when veilid-core is reachable from it through
//! **normal** dependency edges, whether it names veilid-core in its own
//! manifest or not: a tier crate that imports a type from a crate that
//! links veilid-core compiles, versions and can reach all of Veilid. So the
//! check walks the resolved graph from `cargo metadata`, not manifests.
//!
//! Only these may link veilid-core:
//! - `rekindle-protocol`: every Veilid call (record pool, `RouteImports`,
//!   `OwnRoutes`, node startup), placed by rule B22;
//! - `rekindle-transport`: the daemon's adapter over it;
//! - `rekindle-node`: the host;
//! - `rekindle-desktop`, until F1 makes it a thin client, and
//!   `rekindle-e2e-server`, whose binary embeds the desktop's command layer.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;

const ALLOWED: &[&str] = &[
    "rekindle-protocol",
    "rekindle-transport",
    "rekindle-node",
    "rekindle-desktop",
    "rekindle-e2e-server",
];

const TARGET: &str = "veilid-core";

#[derive(Deserialize)]
struct Metadata {
    packages: Vec<Package>,
    workspace_members: Vec<String>,
    resolve: Resolve,
}

#[derive(Deserialize)]
struct Package {
    id: String,
    name: String,
}

#[derive(Deserialize)]
struct Resolve {
    nodes: Vec<Node>,
}

#[derive(Deserialize)]
struct Node {
    id: String,
    deps: Vec<NodeDep>,
}

#[derive(Deserialize)]
struct NodeDep {
    pkg: String,
    dep_kinds: Vec<DepKind>,
}

#[derive(Deserialize)]
struct DepKind {
    /// `None` is a normal dependency; `"dev"` and `"build"` are not linked
    /// into the crate's library.
    kind: Option<String>,
}

/// Package names reachable from `from` through normal edges in `graph`
/// (package name → its normal dependencies' names).
fn reachable<'a>(
    graph: &'a BTreeMap<String, BTreeSet<String>>,
    from: &'a str,
) -> BTreeSet<&'a str> {
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::from([from]);
    while let Some(name) = queue.pop_front() {
        for dep in graph.get(name).into_iter().flatten() {
            if seen.insert(dep.as_str()) {
                queue.push_back(dep.as_str());
            }
        }
    }
    seen
}

/// Every member of `members` outside the allowlist from which `TARGET` is
/// reachable.
fn violations(graph: &BTreeMap<String, BTreeSet<String>>, members: &[String]) -> Vec<String> {
    members
        .iter()
        .filter(|name| !ALLOWED.contains(&name.as_str()))
        .filter(|name| reachable(graph, name).contains(TARGET))
        .cloned()
        .collect()
}

/// The normal-dependency graph by package name, and the workspace members'
/// names, from `cargo metadata`.
fn load(root: &Path) -> Result<(BTreeMap<String, BTreeSet<String>>, Vec<String>)> {
    let output =
        std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
            .args(["metadata", "--format-version", "1"])
            .current_dir(root)
            .output()
            .context("running cargo metadata")?;
    if !output.status.success() {
        return Err(anyhow!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let metadata: Metadata =
        serde_json::from_slice(&output.stdout).context("parsing cargo metadata")?;
    let names: BTreeMap<&str, &str> = metadata
        .packages
        .iter()
        .map(|p| (p.id.as_str(), p.name.as_str()))
        .collect();
    let mut graph: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for node in &metadata.resolve.nodes {
        let Some(name) = names.get(node.id.as_str()) else {
            continue;
        };
        let deps = node
            .deps
            .iter()
            .filter(|dep| dep.dep_kinds.iter().any(|k| k.kind.is_none()))
            .filter_map(|dep| names.get(dep.pkg.as_str()).map(|n| (*n).to_string()));
        graph.entry((*name).to_string()).or_default().extend(deps);
    }
    let members = metadata
        .workspace_members
        .iter()
        .filter_map(|id| names.get(id.as_str()).map(|n| (*n).to_string()))
        .collect();
    Ok((graph, members))
}

pub(crate) fn check_veilid_boundary(root: &Path) -> Result<Vec<String>> {
    let (graph, members) = load(root)?;
    Ok(violations(&graph, &members)
        .into_iter()
        .map(|name| {
            format!(
                "{name}: links `{TARGET}` (transitively); the Veilid boundary is linkage (ADR 0014) \
                 — take wire types from rekindle-codec / rekindle-types, not rekindle-protocol"
            )
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(edges: &[(&str, &[&str])]) -> BTreeMap<String, BTreeSet<String>> {
        edges
            .iter()
            .map(|(from, to)| {
                (
                    (*from).to_string(),
                    to.iter().map(|t| (*t).to_string()).collect(),
                )
            })
            .collect()
    }

    fn members(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| (*n).to_string()).collect()
    }

    #[test]
    fn a_tier_crate_that_reaches_veilid_through_another_crate_fails() {
        // The planted dependency: rekindle-voice → rekindle-protocol → veilid-core.
        let g = graph(&[
            ("rekindle-voice", &["rekindle-codec", "rekindle-protocol"]),
            ("rekindle-protocol", &["veilid-core", "rekindle-codec"]),
            ("rekindle-codec", &["rekindle-types"]),
        ]);
        assert_eq!(
            violations(&g, &members(&["rekindle-voice", "rekindle-protocol"])),
            vec!["rekindle-voice".to_string()]
        );
    }

    #[test]
    fn veilid_free_tier_crates_and_the_allowlist_pass() {
        let g = graph(&[
            ("rekindle-voice", &["rekindle-codec"]),
            ("rekindle-codec", &["rekindle-types"]),
            ("rekindle-protocol", &["veilid-core", "rekindle-codec"]),
            ("rekindle-node", &["rekindle-transport"]),
            ("rekindle-transport", &["rekindle-protocol"]),
        ]);
        assert!(violations(
            &g,
            &members(&[
                "rekindle-voice",
                "rekindle-codec",
                "rekindle-protocol",
                "rekindle-node"
            ])
        )
        .is_empty());
    }

    #[test]
    fn a_cycle_in_the_graph_terminates() {
        let g = graph(&[("a", &["b"]), ("b", &["a", "veilid-core"])]);
        assert_eq!(violations(&g, &members(&["a"])), vec!["a".to_string()]);
    }
}
