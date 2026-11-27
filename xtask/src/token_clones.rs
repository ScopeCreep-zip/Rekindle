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

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;

use crate::walk_source_files;

/// The trees jscpd scans, and the formats it reads in them.
const SCAN: [&str; 4] = ["crates", "src-tauri", "xtask", "src"];
const FORMATS: &str = "rust,typescript,tsx";
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
    for dir in SCAN {
        for path in walk_source_files(&root.join(dir), &["rs", "ts", "tsx", "js", "jsx", "css"])? {
            let rel = path.strip_prefix(root).unwrap_or(&path).display();
            if path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| CONFIG_FILES.contains(&n))
            {
                out.push(format!("{rel}: a jscpd config file can carry ignores"));
            }
            let src = std::fs::read_to_string(&path)?;
            for line in marker_lines(&src) {
                out.push(format!("{rel}:{line}: jscpd ignore marker"));
            }
        }
    }
    Ok(out)
}

/// Runs jscpd over `paths` (relative to `root`) and returns its clones.
pub(crate) fn run_jscpd(root: &Path, paths: &[&str], out_dir: &Path) -> Result<Vec<Duplicate>> {
    let bin = root.join("node_modules/.bin/jscpd");
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
        .args([
            "--ignore",
            "**/target/**,**/node_modules/**,**/gen/**",
            "--absolute",
        ])
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
    let present: Vec<&str> = SCAN.into_iter().filter(|d| root.join(d).exists()).collect();
    let dups = run_jscpd(root, &present, &dir)?;
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
        "    token-clones: {} clones (rust {rust}, ts/tsx {ts}), {} refused suppressions",
        dups.len(),
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
