//! `cargo xtask import-skin --format <f> --from <path> --builtin <id>`
//! (plan E7.7.3): reads a legacy skin tree from disk, imports it with
//! `rekindle-skin::import`, and writes the built-in pack
//! `crates/rekindle-skin/builtin/<id>.rkskin` with its `<id>.NOTICE`.
//! The import is deterministic: the same tree writes the same bytes.
//! `--credit-source`/`--credit-quote` give Xfire's authorship stated outside
//! the files (plan E7.7.3c); `--source-correction` records, in the NOTICE, each
//! line our copy of the pack source changes from the author's (plan SK2 lead
//! decision 54: xSLIMFIRE's popup anchor); `--dry-run` imports and reports,
//! writing nothing.

use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use rekindle_skin::import::{
    miranda, mumble, pidgin, teamspeak, winamp, xfire_ini, xfire_xml, Imported, Outcome, Tree,
    XfireCredit,
};
use walkdir::WalkDir;

/// Where built-in packs live (plan E7.7.1 "Built-in packs").
pub const BUILTIN_DIR: &str = "crates/rekindle-skin/builtin";

/// The legacy formats `--format` takes.
#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum Format {
    /// Xfire XML era (Carbon Fiber, Symbiosis).
    XfireXml,
    /// Xfire INI era (DoiDD): `version.ini`, `<LongName>.ini` and its theme.
    XfireIni,
    /// Winamp classic `.wsz` (`--from` is the archive file).
    WinampWsz,
    /// Pidgin 2.x theme (`purple/{blist,status-icon,sound}/theme.xml`):
    /// `--from` is the theme directory or its `.tgz`/`.tar.gz` archive.
    Pidgin,
    /// Mumble theme directory (`theme.ini`, its QSS `ApplicationPalette`,
    /// PNG/SVG icons).
    Mumble,
    /// TeamSpeak 3 style tree (`styles/<name>.qss` `QPalette::` overrides,
    /// `styles/<name>/media/*.png`).
    Teamspeak3,
    /// Miranda NG skin directory: one TabSRMM `.tsk` or Clist_modern `.msf`
    /// and the images it names.
    Miranda,
}

/// A user-format import as a built-in: a draft's suggestions are accepted,
/// like the walk on every built-in, each change printed and recorded in the
/// NOTICE.
fn accept(outcome: Outcome) -> Result<Imported> {
    Ok(match outcome {
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
    })
}

fn winamp_pack(from: &Path, id: &str) -> Result<Imported> {
    let bytes = std::fs::read(from).with_context(|| format!("reading {}", from.display()))?;
    accept(winamp::import(&bytes, id).map_err(|e| anyhow!("{e}"))?)
}

/// A Pidgin theme from its directory or its `.tgz`/`.tar.gz` archive (the
/// two extensions Pidgin's own theme installer unpacks).
fn pidgin_pack(from: &Path, id: &str) -> Result<Imported> {
    if from.is_dir() {
        return accept(pidgin::import(&read_tree(from)?, id).map_err(|e| anyhow!("{e}"))?);
    }
    let ext = |p: &Path| {
        p.extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
    };
    let archive = match ext(from).as_deref() {
        Some("tgz") => true,
        Some("gz") => from
            .file_stem()
            .is_some_and(|stem| ext(Path::new(stem)).as_deref() == Some("tar")),
        _ => false,
    };
    if !archive {
        bail!(
            "--from {}: a theme directory or a .tgz/.tar.gz archive",
            from.display()
        );
    }
    let bytes = std::fs::read(from).with_context(|| format!("reading {}", from.display()))?;
    accept(pidgin::import_archive(&bytes, id).map_err(|e| anyhow!("{e}"))?)
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

/// The name of the skin folder `dir` (an INI-era skin's file is named by
/// it, `rekindle_skin::import::xfire_ini`), resolved so `.` names it too.
fn folder_name(dir: &Path) -> Result<String> {
    let full =
        std::fs::canonicalize(dir).with_context(|| format!("resolving {}", dir.display()))?;
    full.file_name()
        .and_then(|n| n.to_str())
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("{}: no UTF-8 folder name", dir.display()))
}

fn is_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

/// One `import-skin` run.
pub struct Request<'a> {
    pub format: Format,
    pub from: &'a Path,
    pub id: &'a str,
    /// `(source URL, quote)`: Xfire's authorship stated outside the files.
    pub credit: Option<(&'a str, &'a str)>,
    /// Each correction our copy of the source makes (file, line, before →
    /// after, why), listed in the NOTICE.
    pub corrections: &'a [String],
    pub dry_run: bool,
}

/// Imports `from` as the built-in `id` and writes its pack and NOTICE (or,
/// on a dry run, prints the outcome and writes nothing).
///
/// # Errors
/// An unreadable tree, an id outside `[a-z0-9-]`, a credit that is refused
/// or given to a format that is not Xfire's, or a refused import.
pub fn import_skin(root: &Path, req: &Request<'_>) -> Result<()> {
    let (format, from, id) = (req.format, req.from, req.id);
    if !is_id(id) {
        bail!("--builtin {id}: 1-64 of a-z 0-9 -");
    }
    let credit = match req.credit {
        Some(_) if !matches!(format, Format::XfireXml | Format::XfireIni) => {
            bail!("--credit-source/--credit-quote: only xfire-xml and xfire-ini skins")
        }
        Some((source, quote)) => Some(XfireCredit::new(source, quote).map_err(|e| anyhow!("{e}"))?),
        None => None,
    };
    let Imported {
        canonical, notice, ..
    } = match format {
        Format::XfireXml => {
            xfire_xml::import(&read_tree(from)?, id, credit.as_ref()).map_err(|e| anyhow!("{e}"))?
        }
        Format::XfireIni => accept(
            xfire_ini::import(&read_tree(from)?, &folder_name(from)?, id, credit.as_ref())
                .map_err(|e| anyhow!("{e}"))?,
        )?,
        Format::WinampWsz => winamp_pack(from, id)?,
        Format::Pidgin => pidgin_pack(from, id)?,
        Format::Mumble => {
            accept(mumble::import(&read_tree(from)?, id).map_err(|e| anyhow!("{e}"))?)?
        }
        Format::Teamspeak3 => {
            accept(teamspeak::import(&read_tree(from)?, id).map_err(|e| anyhow!("{e}"))?)?
        }
        Format::Miranda => {
            accept(miranda::import(&read_tree(from)?, id).map_err(|e| anyhow!("{e}"))?)?
        }
    };
    if req.dry_run {
        println!(
            "    dry run: {id} imports ({} bytes, {}); nothing written",
            canonical.bytes.len(),
            canonical.address()
        );
        return Ok(());
    }
    let dir = root.join(BUILTIN_DIR);
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let pack = dir.join(format!("{id}.rkskin"));
    std::fs::write(&pack, &canonical.bytes)
        .with_context(|| format!("writing {}", pack.display()))?;
    let notice_path = dir.join(format!("{id}.NOTICE"));
    std::fs::write(&notice_path, with_corrections(notice, req.corrections))
        .with_context(|| format!("writing {}", notice_path.display()))?;
    println!(
        "    wrote {BUILTIN_DIR}/{id}.rkskin ({})",
        canonical.address()
    );
    println!("    wrote {BUILTIN_DIR}/{id}.NOTICE");
    Ok(())
}

/// `notice` with a section listing `corrections`, when there are any.
fn with_corrections(mut notice: String, corrections: &[String]) -> String {
    if !corrections.is_empty() {
        notice.push_str(
            "\nSource corrections (our copy of the pack source differs from the author's)\n",
        );
        for c in corrections {
            notice.push_str("  ");
            notice.push_str(c.trim());
            notice.push('\n');
        }
    }
    notice
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_source_correction_is_listed_in_the_notice() {
        assert_eq!(with_corrections("N\n".to_owned(), &[]), "N\n");
        let got = with_corrections("N\n".to_owned(), &["a.xml:5: x -> y".to_owned()]);
        assert!(
            got.ends_with("differs from the author's)\n  a.xml:5: x -> y\n"),
            "{got}"
        );
    }

    #[test]
    fn ids_are_checked() {
        assert!(is_id("carbon-fiber"));
        assert!(!is_id("../x"));
        assert!(!is_id("Carbon"));
        assert!(!is_id(""));
    }

    #[test]
    fn a_pidgin_file_must_be_a_tgz() {
        let e = pidgin_pack(Path::new("theme.zip"), "x").unwrap_err();
        assert!(e.to_string().contains(".tgz/.tar.gz"), "{e}");
    }
}
