//! The resolved workspace from `cargo metadata`, for the gates that must see
//! the dependency graph as cargo resolves it (a renamed dependency, a
//! workspace dependency, a dependency inside a macro-built file) rather than
//! as the manifests spell it.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;

#[derive(Deserialize)]
pub(crate) struct Metadata {
    pub(crate) packages: Vec<Package>,
    pub(crate) resolve: Resolve,
}

#[derive(Deserialize)]
pub(crate) struct Package {
    pub(crate) id: String,
    pub(crate) name: String,
}

#[derive(Deserialize)]
pub(crate) struct Resolve {
    pub(crate) nodes: Vec<Node>,
}

#[derive(Deserialize)]
pub(crate) struct Node {
    pub(crate) id: String,
    pub(crate) deps: Vec<NodeDep>,
}

#[derive(Deserialize)]
pub(crate) struct NodeDep {
    pub(crate) pkg: String,
}

impl Metadata {
    /// `cargo metadata` for the workspace at `root`.
    pub(crate) fn load(root: &Path) -> Result<Self> {
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
        serde_json::from_slice(&output.stdout).context("parsing cargo metadata")
    }

    fn name_of(&self, id: &str) -> Option<&str> {
        self.packages
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.name.as_str())
    }

    /// Every package with a direct dependency of any kind on `name`.
    pub(crate) fn direct_dependents(&self, name: &str) -> BTreeSet<String> {
        self.resolve
            .nodes
            .iter()
            .filter(|n| n.deps.iter().any(|d| self.name_of(&d.pkg) == Some(name)))
            .filter_map(|n| self.name_of(&n.id).map(str::to_owned))
            .collect()
    }
}
