//! Workspace root discovery and source-file walking shared by every check.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use ignore::WalkBuilder;

pub(crate) fn workspace_root() -> Result<PathBuf> {
    // CARGO_MANIFEST_DIR is set when invoked via `cargo run -p xtask`.
    // Walk up until we find the workspace Cargo.toml.
    let mut p = std::env::current_dir().context("getting current dir")?;
    loop {
        let cargo = p.join("Cargo.toml");
        if cargo.exists() {
            let txt = std::fs::read_to_string(&cargo)?;
            if txt.contains("[workspace]") {
                return Ok(p);
            }
        }
        if !p.pop() {
            return Err(anyhow!("could not find workspace root"));
        }
    }
}

/// Walk `dir` for files with any of `exts`, honouring .gitignore and
/// skipping build/dependency trees.
pub(crate) fn walk_source_files(dir: &Path, exts: &[&str]) -> Result<Vec<PathBuf>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let walker = WalkBuilder::new(dir)
        .standard_filters(true)
        .filter_entry(|e| {
            !e.path()
                .components()
                .any(|c| matches!(c.as_os_str().to_str(), Some("target" | "node_modules")))
        })
        .build();
    for entry in walker {
        let entry = entry?;
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.path();
        if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| exts.contains(&e))
        {
            out.push(path.to_path_buf());
        }
    }
    Ok(out)
}
