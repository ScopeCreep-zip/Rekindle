//! The workspace source-file walk the checks share (moved whole from
//! `main.rs`).

use std::path::{Path, PathBuf};

use anyhow::Result;
use ignore::WalkBuilder;

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
