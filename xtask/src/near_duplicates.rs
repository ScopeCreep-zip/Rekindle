//! `cargo xtask check-near-duplicates` (gate hardening item 2, owner
//! 2026-11-26: "we NEVER should have dupe code"): the `syn` clone pass of
//! `research-xtask-dup-modules-2026-11-26.md` §1.4 and §5.2 item 1.
//!
//! The byte-identical checks (`check-duplicate-bodies`,
//! `check-frontend-duplication`) let a reshaped port through: renamed
//! identifiers, edited statements and split functions leave no identical
//! body. This pass normalises every function body (locals and item names
//! renamed consistently, literals erased to their kind, paths cut to their
//! last segment, attributes and comments dropped; `tokens.rs`) and compares:
//!
//! - **functions**: the Jaccard index of their `SHINGLE`-token window sets
//!   (Type-2 and Type-3 function clones);
//! - **statement runs**: any run of `RUN_STMTS` consecutive statements that
//!   is identical after renaming in two places (fragments, split or merged
//!   functions);
//! - **file pairs**: the Jaccard index of the two files' shingle sets (a
//!   ported module).
//!
//! Scope: first-party code (`crate::first_party`, the lead's ruling of
//! 2026-11-26 "Item 2 rulings after the triage"): every first-party `.rs`
//! file under `crates/`, `src-tauri/` and `xtask/`, tests included (rule 1
//! has no test exemption). A vendored third-party crate is outside by that
//! definition. Generated code lives in `OUT_DIR`, outside the walked trees.
//! No baseline, no list.
//!
//! Calibration (2026-11-26): the floors and thresholds below were set so the
//! E4.1 S5 voice-adapter port, reproduced from history in
//! `xtask/fixtures/near_duplicates/`, fails as a file pair (its similarity is
//! asserted against `FILE_SIMILARITY` in the tests), while an unrelated
//! module and two same-domain modules that are not a port pass. Shingle sizes
//! 6, 8, 10, 12 and 16 were measured; 12 separates the port from the
//! same-domain pairs best (≈ 4.7×).
//!
//! Calibration (ruling of 2026-11-26 after the triage): `use` items are
//! dropped with the attributes (`tokens.rs`), and a function, a statement
//! run or a function feeding a file pair is compared only when it holds at
//! least `MIN_STRUCTURAL` structural tokens (`crate::structural`), so a list
//! of paths, variants or lints never matches another list.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use anyhow::{anyhow, Result};

use crate::first_party::first_party_files;
use crate::structural::MIN_STRUCTURAL;

mod compare;
mod tokens;
mod units;

use compare::Parsed;

/// The comparison floors and thresholds.
pub(crate) struct Thresholds {
    pub(crate) shingle: usize,
    pub(crate) min_fn_tokens: usize,
    pub(crate) fn_similarity: f64,
    pub(crate) run_stmts: usize,
    pub(crate) min_run_tokens: usize,
    pub(crate) min_file_shared: usize,
    pub(crate) file_similarity: f64,
    pub(crate) min_structural: usize,
}

/// Tokens per shingle window.
const SHINGLE: usize = 12;
/// The smallest function body compared (jscpd's default floor, 50 tokens,
/// plus the body's braces and a statement).
const MIN_FN_TOKENS: usize = 60;
/// The lowest function-pair Jaccard index reported (≈ Dice 0.82, the
/// dupes-core and jscpd `--similarity` default of 0.8).
const FN_SIMILARITY: f64 = 0.7;
/// Statements per run (research §1.4 step 3, k ≈ 4).
const RUN_STMTS: usize = 4;
/// The smallest statement run compared (jscpd's `--min-tokens` default).
const MIN_RUN_TOKENS: usize = 50;
/// The fewest shingles a file pair must share (the S5 port's pairs share
/// 251 and 321; two same-domain modules that are not a port, 55 and 75).
const MIN_FILE_SHARED: usize = 150;
/// The lowest file-pair Jaccard index reported. At `SHINGLE` = 12 the S5
/// port's two file pairs score 0.115 (deps) and 0.122 (session setup), two
/// same-domain modules that are not a port 0.024 and 0.025, an unrelated
/// module 0.003 (the research's 0.3 was for a different set: function and
/// window fingerprints, which a reshaped port does not share).
const FILE_SIMILARITY: f64 = 0.08;

/// The gate's thresholds.
pub(crate) const THRESHOLDS: Thresholds = Thresholds {
    shingle: SHINGLE,
    min_fn_tokens: MIN_FN_TOKENS,
    fn_similarity: FN_SIMILARITY,
    run_stmts: RUN_STMTS,
    min_run_tokens: MIN_RUN_TOKENS,
    min_file_shared: MIN_FILE_SHARED,
    file_similarity: FILE_SIMILARITY,
    min_structural: MIN_STRUCTURAL,
};

/// One side of a clone.
pub(crate) struct Loc {
    pub(crate) file: String,
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) what: String,
}

/// One clone pair.
pub(crate) struct Finding {
    /// `function`, `statements` or `file`.
    pub(crate) kind: &'static str,
    pub(crate) a: Loc,
    pub(crate) b: Loc,
    /// The smaller side's tokens (shared shingles for a file pair).
    pub(crate) tokens: usize,
    pub(crate) similarity: f64,
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {}:{}-{} ({}) <> {}:{}-{} ({}) tokens={} similarity={:.2}",
            self.kind,
            self.a.file,
            self.a.start,
            self.a.end,
            self.a.what,
            self.b.file,
            self.b.start,
            self.b.end,
            self.b.what,
            self.tokens,
            self.similarity
        )
    }
}

/// Every clone pair among `sources` (path → text) at `t`. Files `syn`
/// cannot parse are returned in the error.
pub(crate) fn scan(sources: &BTreeMap<String, String>, t: &Thresholds) -> Result<Vec<Finding>> {
    let mut int = tokens::Interner::default();
    let mut files = Vec::new();
    let mut unparsed = Vec::new();
    for (path, src) in sources {
        match units::file_units(src, &mut int) {
            Some(units) => files.push(Parsed {
                path: path.clone(),
                units,
            }),
            None => unparsed.push(path.clone()),
        }
    }
    if !unparsed.is_empty() {
        return Err(anyhow!("syn could not parse: {}", unparsed.join(", ")));
    }
    let mut out = Vec::new();
    let fn_pairs = compare::function_pairs(&files, t, &mut out);
    compare::statement_runs(&files, t, &fn_pairs, &mut out);
    compare::file_pairs(&files, t, &mut out);
    Ok(out)
}

/// Runs the pass over the workspace's Rust sources.
pub(crate) fn check_near_duplicates(root: &Path) -> Result<()> {
    let mut sources = BTreeMap::new();
    for file in first_party_files(root, &["crates", "src-tauri", "xtask"], &["rs"])? {
        sources.insert(file.rel, std::fs::read_to_string(&file.path)?);
    }
    let findings = scan(&sources, &THRESHOLDS)?;
    let mut by_kind: BTreeMap<&str, usize> = BTreeMap::new();
    for f in &findings {
        println!("    {f}");
        *by_kind.entry(f.kind).or_default() += 1;
    }
    println!(
        "    near-duplicates: {} files, {} clone pairs {:?}",
        sources.len(),
        findings.len(),
        by_kind
    );
    if findings.is_empty() {
        Ok(())
    } else {
        Err(anyhow!(
            "{} near-duplicate clone pair(s); merge each into one shared item (no list, no baseline)",
            findings.len()
        ))
    }
}

#[cfg(test)]
#[path = "near_duplicates_tests.rs"]
mod tests;
