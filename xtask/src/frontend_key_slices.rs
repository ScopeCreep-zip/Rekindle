//! Frontend identity labels come from the Host.

use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::walk_source_files;

pub(crate) fn check(root: &Path) -> Result<()> {
    let mut offenders = 0;
    for path in walk_source_files(&root.join("src"), &["ts", "tsx"])? {
        let source = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        for (start, rest) in source.match_indices(".slice") {
            let arguments = source[start + rest.len()..]
                .chars()
                .filter(|c| !c.is_whitespace());
            if !arguments.take(5).eq("(0,8)".chars()) {
                continue;
            }
            let receiver = source[..start]
                .trim_end()
                .rsplit(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '$')
                .next();
            // Channel and media stream IDs identify objects.
            if matches!(receiver, Some("channelId" | "streamId")) {
                continue;
            }
            let line = source[..start]
                .bytes()
                .filter(|byte| *byte == b'\n')
                .count()
                + 1;
            eprintln!("{}:{line}", path.strip_prefix(root)?.display());
            offenders += 1;
        }
    }
    if offenders > 0 {
        bail!(
            "{offenders} frontend slices may truncate a participant key; use the Host's shown name"
        );
    }
    Ok(())
}
