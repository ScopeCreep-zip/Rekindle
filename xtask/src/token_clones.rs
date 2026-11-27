//! `cargo xtask check-token-clones` (gate hardening item 2): the pinned
//! jscpd 5 binary over Rust and TS/TSX, research
//! `research-xtask-dup-modules-2026-11-26.md` §1.3 and §5.2 item 2.
//!
//! jscpd is a devDependency pinned exactly in `package.json` (lockfile
//! `pnpm-lock.yaml`), run from `node_modules/.bin` the way
//! `check-supply-chain` runs `cargo deny`: a missing tool fails the check
//! (`pnpm install`). Its library crates are 0.1.x and called unstable, so it
//! runs as a binary (research §1.2).
//!
//! Flags: `--ignore-identifiers=consistent` (Type-2 under a consistent
//! renaming), `--max-gap-lines 2` (Type-3: clones of one file pair split by
//! at most two edited lines merge), `--similarity 0.85` (whole-function
//! syntax-tree similarity), and the default floors `--min-tokens 50` /
//! `--min-lines 5`, at which the E4.1 S5 voice-adapter port reports seven
//! cross-file clones (calibrated 2026-11-26). No baseline, no
//! `--ignore-pattern`, no config file: every source comment that would
//! silence jscpd (its ignore marker) and any jscpd config file fail the
//! check, because a marker in source is an allow-list written in source
//! (research §1.3).
//!
//! Scope: first-party code (`crate::first_party`, the lead's ruling of
//! 2026-11-26 "Item 2 rulings after the triage"); a vendored third-party
//! crate is outside by that definition.
//!
//! Calibration (same ruling): jscpd is fed the stripped form. The scoped
//! files are mirrored under the report directory, each Rust file with its
//! `use` items and attributes blanked (`strip.rs`, lines kept), and jscpd
//! reads only that mirror, so it never sees an import or a lint list. A Rust
//! clone then counts only when its span in what jscpd read holds at least
//! `MIN_STRUCTURAL` structural tokens on both sides (`crate::structural`);
//! the clones under the floor are counted and printed as such, never hidden.
//! TS/TSX is read as written. Its list class (lead ruling 2026-11-26):
//! a clone whose two sides, in two different files, are each only
//! independent single-variable declarations is a list match, the TS form of
//! the Rust import and lint lists; the same run repeated in one file stays a
//! finding (a missing object). See `ts_lists.rs`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;

use crate::first_party::first_party_files;
use crate::structural::{LexedText, MIN_STRUCTURAL};

mod strip;
mod ts_lists;

pub(crate) use strip::strip_rust;

/// The trees jscpd scans, and the formats it reads in them.
const SCAN: [&str; 4] = ["crates", "src-tauri", "xtask", "src"];
const FORMATS: &str = "rust,typescript,tsx";
/// The extensions mirrored for jscpd.
const MIRRORED: [&str; 3] = ["rs", "ts", "tsx"];
/// jscpd's ignore marker, written split so this file holds no marker.
const MARKER: &str = concat!("jscpd", ":ignore");
/// jscpd's config files, which can carry ignores.
const CONFIG_FILES: [&str; 2] = [".jscpd.json", ".jscpdrc"];

#[derive(Deserialize)]
struct Report {
    duplicates: Vec<Duplicate>,
}

#[derive(Deserialize)]
pub(crate) struct Duplicate {
    pub(crate) format: String,
    #[serde(default)]
    pub(crate) kind: Option<String>,
    #[serde(default)]
    pub(crate) similarity: Option<f64>,
    pub(crate) lines: usize,
    pub(crate) tokens: usize,
    #[serde(rename = "firstFile")]
    pub(crate) first: Side,
    #[serde(rename = "secondFile")]
    pub(crate) second: Side,
}

#[derive(Deserialize)]
pub(crate) struct Side {
    pub(crate) name: String,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

impl std::fmt::Display for Duplicate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} {} {}:{}-{} <> {}:{}-{} lines={} tokens={}",
            self.format,
            self.kind.as_deref().unwrap_or("clone"),
            self.first.name,
            self.first.start,
            self.first.end,
            self.second.name,
            self.second.start,
            self.second.end,
            self.lines,
            self.tokens
        )?;
        if let Some(s) = self.similarity {
            write!(f, " similarity={s:.2}")?;
        }
        Ok(())
    }
}

/// Parses a jscpd JSON report.
pub(crate) fn parse_report(json: &str) -> Result<Vec<Duplicate>> {
    let report: Report = serde_json::from_str(json).context("parsing the jscpd JSON report")?;
    Ok(report.duplicates)
}

/// The lines of `src` that carry jscpd's ignore marker (1-based).
pub(crate) fn marker_lines(src: &str) -> Vec<usize> {
    src.lines()
        .enumerate()
        .filter(|(_, l)| l.contains(MARKER))
        .map(|(i, _)| i + 1)
        .collect()
}

/// Every ignore marker and jscpd config file under `root`'s scanned trees.
pub(crate) fn refused_suppressions(root: &Path) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for name in CONFIG_FILES {
        if root.join(name).exists() {
            out.push(format!("{name}: a jscpd config file can carry ignores"));
        }
    }
    for file in first_party_files(root, &SCAN, &["rs", "ts", "tsx", "js", "jsx", "css"])? {
        let (path, rel) = (&file.path, &file.rel);
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| CONFIG_FILES.contains(&n))
        {
            out.push(format!("{rel}: a jscpd config file can carry ignores"));
        }
        let src = std::fs::read_to_string(path)?;
        for line in marker_lines(&src) {
            out.push(format!("{rel}:{line}: jscpd ignore marker"));
        }
    }
    Ok(out)
}

/// Mirrors the first-party files of `trees` under `input`, Rust stripped.
fn mirror(root: &Path, trees: &[&str], input: &Path) -> Result<()> {
    if input.exists() {
        std::fs::remove_dir_all(input)?;
    }
    for file in first_party_files(root, trees, &MIRRORED)? {
        let src = std::fs::read_to_string(&file.path)?;
        let text = if file.path.extension().is_some_and(|e| e == "rs") {
            strip_rust(&src)
        } else {
            src
        };
        let dest = input.join(&file.rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(dest, text)?;
    }
    Ok(())
}

/// The lines `side` spans in `text`.
fn span_text(text: &str, side: &Side) -> String {
    text.lines()
        .skip(side.start.saturating_sub(1))
        .take((side.end + 1).saturating_sub(side.start))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Splits the list matches from the clones: a Rust clone under the
/// structural floor, or a cross-file TS/TSX clone whose sides are both
/// independent declarations. `input` holds the text jscpd read, under each
/// clone's (relative) name.
pub(crate) fn split_lists(
    input: &Path,
    dups: Vec<Duplicate>,
) -> Result<(Vec<Duplicate>, Vec<Duplicate>)> {
    let mut lexed: HashMap<String, LexedText> = HashMap::new();
    let mut structural = |side: &Side| -> Result<usize> {
        if !lexed.contains_key(&side.name) {
            let src = std::fs::read_to_string(input.join(&side.name))?;
            lexed.insert(side.name.clone(), LexedText::new(&src));
        }
        Ok(lexed[&side.name].structural(side.start, side.end))
    };
    let (mut kept, mut under) = (Vec::new(), Vec::new());
    for d in dups {
        let list = if d.format == "rust" {
            structural(&d.first)?.min(structural(&d.second)?) < MIN_STRUCTURAL
        } else {
            let decls = |side: &Side| -> Result<bool> {
                let text = std::fs::read_to_string(input.join(&side.name))?;
                Ok(ts_lists::independent_declarations(&span_text(&text, side)))
            };
            d.first.name != d.second.name && decls(&d.first)? && decls(&d.second)?
        };
        if list {
            under.push(d);
        } else {
            kept.push(d);
        }
    }
    Ok((kept, under))
}

/// Runs the workspace's jscpd over `paths` (relative to `root`, the tree
/// jscpd reads) and returns its clones, named relative to `root`.
pub(crate) fn run_jscpd(
    workspace: &Path,
    root: &Path,
    paths: &[&str],
    out_dir: &Path,
) -> Result<Vec<Duplicate>> {
    let bin = workspace.join("node_modules/.bin/jscpd");
    let status = Command::new(&bin)
        .args(["--ignore-identifiers=consistent", "--max-gap-lines", "2"])
        .args([
            "--similarity=0.85",
            "--min-tokens",
            "50",
            "--min-lines",
            "5",
        ])
        .args([
            "--format",
            FORMATS,
            "--reporters",
            "json",
            "--report-name",
            "token-clones",
        ])
        .arg("--absolute")
        .arg("--output")
        .arg(out_dir)
        .args(paths)
        .current_dir(root)
        .status()
        .with_context(|| format!("running {} (run `pnpm install`)", bin.display()))?;
    if !status.success() {
        return Err(anyhow!("jscpd failed ({status}); see its output above"));
    }
    let mut dups = parse_report(&std::fs::read_to_string(out_dir.join("token-clones.json"))?)?;
    // `--absolute` names files from `/`; report them from the root.
    let prefix = format!("{}/", root.display());
    for d in &mut dups {
        for side in [&mut d.first, &mut d.second] {
            if let Some(rel) = side.name.strip_prefix(&prefix) {
                side.name = rel.to_owned();
            }
        }
    }
    Ok(dups)
}

fn report_dir(root: &Path) -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map_or_else(|| root.join("target"), PathBuf::from)
        .join("xtask-token-clones")
}

/// Refuses every suppression, then runs jscpd over the scanned trees.
pub(crate) fn check_token_clones(root: &Path) -> Result<()> {
    let refused = refused_suppressions(root)?;
    for r in &refused {
        println!("    {r}");
    }
    let dir = report_dir(root);
    std::fs::create_dir_all(&dir)?;
    let input = dir.join("input");
    mirror(root, &SCAN, &input)?;
    let present: Vec<&str> = SCAN
        .into_iter()
        .filter(|d| input.join(d).exists())
        .collect();
    let (dups, under) = split_lists(&input, run_jscpd(root, &input, &present, &dir)?)?;
    for d in &under {
        println!("    list match (not a clone): {d}");
    }
    let (mut rust, mut ts) = (0usize, 0usize);
    for d in &dups {
        println!("    {d}");
        if d.format == "rust" {
            rust += 1;
        } else {
            ts += 1;
        }
    }
    println!(
        "    token-clones: {} clones (rust {rust}, ts/tsx {ts}), {} list matches, {} refused suppressions",
        dups.len(),
        under.len(),
        refused.len()
    );
    if dups.is_empty() && refused.is_empty() {
        Ok(())
    } else {
        Err(anyhow!(
            "{} jscpd clone(s), {} refused suppression(s)",
            dups.len(),
            refused.len()
        ))
    }
}

#[cfg(test)]
#[path = "token_clones_tests.rs"]
mod tests;
