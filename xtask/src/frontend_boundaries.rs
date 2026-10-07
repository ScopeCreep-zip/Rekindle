//! `check-frontend-boundaries`: no frontend reaches the backend (ADR 0010).
//!
//! The frontends are clients of `rekindled` over the bus. If one of them
//! links Veilid, the transport, the vault or the node (directly or through
//! any normal dependency), it can bypass the daemon and the boundary is
//! gone. A manifest scan only sees direct dependencies, so this walks the
//! resolved graph from `cargo metadata`, following normal edges only:
//! dev- and build-dependencies never ship in the binary.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;

/// The frontend crates (plan C3; the desktop joins at F1).
const FRONTENDS: &[&str] = &["rekindle-cli", "rekindle-tui", "rekindle-client"];

/// What a frontend must never link.
const BACKEND: &[&str] = &[
    "veilid-core",
    "rekindle-transport",
    "rekindle-protocol",
    "rekindle-vault",
    "rekindle-db",
    "rekindle-node",
    "rekindle-desktop",
];

#[derive(Deserialize)]
struct Metadata {
    packages: Vec<Package>,
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
    deps: Vec<Dep>,
}

#[derive(Deserialize)]
struct Dep {
    pkg: String,
    dep_kinds: Vec<DepKind>,
}

#[derive(Deserialize)]
struct DepKind {
    /// `None` is a normal dependency; `"dev"` and `"build"` do not ship.
    kind: Option<String>,
}

pub fn check_frontend_boundaries(root: &Path) -> Result<()> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--locked"])
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
    let violations = violations(&metadata)?;
    if violations.is_empty() {
        return Ok(());
    }
    eprintln!("Frontend-boundary violations (ADR 0010):");
    for v in &violations {
        eprintln!("  • {v}");
    }
    Err(anyhow!(
        "{} frontend-boundary violation(s)",
        violations.len()
    ))
}

fn violations(metadata: &Metadata) -> Result<Vec<String>> {
    let names: HashMap<&str, &str> = metadata
        .packages
        .iter()
        .map(|p| (p.id.as_str(), p.name.as_str()))
        .collect();
    let normal_edges: HashMap<&str, Vec<&str>> = metadata
        .resolve
        .nodes
        .iter()
        .map(|node| {
            let deps = node
                .deps
                .iter()
                .filter(|d| d.dep_kinds.iter().any(|k| k.kind.is_none()))
                .map(|d| d.pkg.as_str())
                .collect();
            (node.id.as_str(), deps)
        })
        .collect();

    let mut found = Vec::new();
    for frontend in FRONTENDS {
        let start = names
            .iter()
            .find(|(_, name)| *name == frontend)
            .map(|(id, _)| *id)
            .ok_or_else(|| anyhow!("frontend crate `{frontend}` is not in the workspace"))?;
        // Depth-first over normal edges, remembering one path per package.
        let mut seen = BTreeSet::new();
        let mut stack = vec![(start, vec![*frontend])];
        while let Some((id, path)) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            for dep in normal_edges.get(id).into_iter().flatten() {
                let name = names.get(dep).copied().unwrap_or("?");
                let mut next = path.clone();
                next.push(name);
                if BACKEND.contains(&name) {
                    found.push(next.join(" → "));
                } else {
                    stack.push((dep, next));
                }
            }
        }
    }
    found.sort();
    found.dedup();
    Ok(found)
}
