//! `check-file-sizes`: one line ceiling for every source file.

use std::path::Path;

use anyhow::{anyhow, Result};

use crate::walk_source_files;

/// One ceiling for every source file, Rust and frontend alike. Any
/// file over it fails the gate — no allowlist, no escape hatch. A
/// deliberate future exception would require editing this check in a
/// reviewed PR, which is exactly the friction an exception deserves.
const MAX_LINES: usize = 600;

pub(crate) fn check_file_sizes(root: &Path) -> Result<()> {
    let mut offenders = 0usize;

    // `css` is in here because a 7,926-line xfire-theme.css was the one
    // source file the ceiling could not see. It is now split under
    // src/styles/theme/, and this keeps it that way.
    for (subdir, exts) in [
        ("src", &["ts", "tsx", "css"][..]),
        ("src-tauri/src", &["rs"][..]),
        ("crates", &["rs"][..]),
        ("xtask", &["rs"][..]),
    ] {
        for path in walk_source_files(&root.join(subdir), exts)? {
            let lines = std::fs::read_to_string(&path)
                .map(|s| s.lines().count())
                .unwrap_or(0);
            if lines <= MAX_LINES {
                continue;
            }
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .display()
                .to_string();
            println!("  ✗ {rel} ({lines} lines, ceiling {MAX_LINES})");
            offenders += 1;
        }
    }

    if offenders > 0 {
        return Err(anyhow!(
            "{offenders} file(s) over {MAX_LINES} lines.\n\
             Split the file along a module seam (see docs/contributor/architecture-rules.md)."
        ));
    }
    Ok(())
}
