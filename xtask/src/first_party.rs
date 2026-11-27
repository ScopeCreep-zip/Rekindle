//! The scope of the authorship checks (`check-near-duplicates`,
//! `check-token-clones`): first-party code (gate hardening item 2, "Item 2
//! rulings after the triage", lead 2026-11-26).
//!
//! First-party code is every source file the project writes under a scanned
//! tree. A third-party crate vendored under `crates/vendor/` (today
//! `tauri-plugin-wdio-webdriver` 1.5.1, MIT, patched with a request guard)
//! keeps its upstream licence and shape: it is governed by `cargo vet` (item
//! 3(e)) and the minimal-patch rule recorded in its NOTICE, not by our
//! authorship checks. This rule defines the scope; it is not an exception
//! list, and no first-party file can be placed outside it.

use std::path::Path;

use anyhow::Result;

use crate::walk::{files_under, SourcePath};

/// Where vendored third-party crates live.
const THIRD_PARTY: &str = "crates/vendor/";

/// The first-party files with any of `exts` under `dirs` (relative to
/// `root`).
pub(crate) fn first_party_files(
    root: &Path,
    dirs: &[&str],
    exts: &[&str],
) -> Result<Vec<SourcePath>> {
    let mut out = Vec::new();
    for dir in dirs {
        out.extend(
            files_under(root, &root.join(dir), exts)?
                .into_iter()
                .filter(|f| !f.rel.starts_with(THIRD_PARTY)),
        );
    }
    Ok(out)
}
