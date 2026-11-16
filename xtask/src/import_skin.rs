//! `cargo xtask import-skin --format <f> --from <path> --builtin <id>`
//! (plan E7.7.3): reads a legacy skin tree from disk, imports it with
//! `rekindle-skin::import`, and writes the built-in pack
//! `crates/rekindle-skin/builtin/<id>.rkskin` with its `<id>.NOTICE`.
//! The import is deterministic: the same tree writes the same bytes.

use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use rekindle_skin::import::winamp::{self, Outcome};
use rekindle_skin::import::{xfire_xml, Imported, Tree};
use walkdir::WalkDir;

/// Where built-in packs live (plan E7.7.1 "Built-in packs").
pub const BUILTIN_DIR: &str = "crates/rekindle-skin/builtin";

/// The legacy formats `--format` takes.
#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum Format {
    /// Xfire XML era (Carbon Fiber, Symbiosis).
    XfireXml,
    /// Winamp classic `.wsz` (`--from` is the archive file).
    WinampWsz,
}

/// A `.wsz` as a built-in: a draft's suggestions are accepted, like the
/// walk on every built-in, each change printed and recorded in the NOTICE.
fn winamp_pack(from: &Path, id: &str) -> Result<Imported> {
    let bytes = std::fs::read(from).with_context(|| format!("reading {}", from.display()))?;
    Ok(
        match winamp::import(&bytes, id).map_err(|e| anyhow!("{e}"))? {
            Outcome::Passed(i) => i,
            Outcome::Draft(d) => {
                for c in &d.failures {
                    println!(
                        "    draft: {} {} on {} fails {}; suggested {:?} -> {:?}",
                        c.fg, c.rule, c.ground, c.token, c.original, c.new
                    );
                }
                match (d.suggested, d.unsettled) {
                    (Some(i), _) => i,
                    (None, Some(u)) => bail!("the walk cannot settle {}: {}", u.error, u.why),
                    (None, None) => bail!("a draft with neither a suggestion nor a failure"),
                }
            }
        },
    )
}

/// Every regular file under `dir`, keyed by its `/`-separated relative path.
fn read_tree(dir: &Path) -> Result<Tree> {
    let mut tree = Tree::new();
    for entry in WalkDir::new(dir).follow_links(false).sort_by_file_name() {
        let entry = entry.with_context(|| format!("walking {}", dir.display()))?;
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = entry.path().strip_prefix(dir)?;
        let parts: Vec<&str> = rel
            .components()
            .map(|c| {
                c.as_os_str()
                    .to_str()
                    .ok_or_else(|| anyhow!("non-UTF-8 path {}", rel.display()))
            })
            .collect::<Result<_>>()?;
        let bytes =
            std::fs::read(entry.path()).with_context(|| format!("reading {}", rel.display()))?;
        tree.insert(parts.join("/"), bytes);
    }
    Ok(tree)
}

fn is_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

/// Imports `from` as the built-in `id` and writes its pack and NOTICE.
///
/// # Errors
/// An unreadable tree, an id outside `[a-z0-9-]`, or a refused import.
pub fn import_skin(root: &Path, format: Format, from: &Path, id: &str) -> Result<()> {
    if !is_id(id) {
        bail!("--builtin {id}: 1-64 of a-z 0-9 -");
    }
    let Imported {
        canonical, notice, ..
    } = match format {
        Format::XfireXml => xfire_xml::import(&read_tree(from)?, id).map_err(|e| anyhow!("{e}"))?,
        Format::WinampWsz => winamp_pack(from, id)?,
    };
    let dir = root.join(BUILTIN_DIR);
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let pack = dir.join(format!("{id}.rkskin"));
    std::fs::write(&pack, &canonical.bytes)
        .with_context(|| format!("writing {}", pack.display()))?;
    let notice_path = dir.join(format!("{id}.NOTICE"));
    std::fs::write(&notice_path, notice)
        .with_context(|| format!("writing {}", notice_path.display()))?;
    println!(
        "    wrote {BUILTIN_DIR}/{id}.rkskin ({})",
        canonical.address()
    );
    println!("    wrote {BUILTIN_DIR}/{id}.NOTICE");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_checked() {
        assert!(is_id("carbon-fiber"));
        assert!(!is_id("../x"));
        assert!(!is_id("Carbon"));
        assert!(!is_id(""));
    }
}
