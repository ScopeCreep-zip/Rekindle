//! `cargo xtask perf`: the wall-clock reports, run by hand (FLK3).
//!
//! A wall-clock bound fails under machine load, so it never gates: each one
//! is an `#[ignore]`d test whose reason names `cargo xtask perf`, and the
//! gating test beside it counts operations instead (e.g.
//! `rekindle-skin/src/serve_tests.rs`). This runs exactly those ignored
//! tests in release, one test binary at a time so no sibling test loads the
//! CPU, prints each one's libtest lines (the result line carries its time;
//! a failed bound's panic names the time it took), and fails if any fails.
//! It is a report, never part of `cargo xtask check`.
//!
//! [`REPORTS`] is the table; the test below holds it equal to the ignored
//! tests in the tree, so a new wall-clock test cannot be left out.

use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{anyhow, Context, Result};

/// One ignored wall-clock test.
pub(crate) struct Report {
    /// The package (`-p`).
    pub krate: &'static str,
    /// The integration-test target (`--test`).
    pub target: &'static str,
    /// The test function, run with `--exact`.
    pub test: &'static str,
    /// What the bound holds and why it is wall clock.
    pub reason: &'static str,
}

/// Every `#[ignore]`d test whose reason names `cargo xtask perf`.
pub(crate) const REPORTS: &[Report] = &[Report {
    krate: "rekindle-skin",
    target: "serve_budget",
    test: "every_builtin_serves_its_stylesheet_in_time",
    reason: "each built-in's stylesheet is served in 1.5 s, inside the client's 5 s timeout \
             (the work itself is gated by operation counts in serve_tests.rs)",
}];

/// Run every report in release, print its lines, and fail if any failed.
pub(crate) fn perf(root: &Path) -> Result<()> {
    let mut failed = Vec::new();
    for r in REPORTS {
        println!("\n── perf: {}::{} ({})", r.krate, r.test, r.reason);
        let out = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
            .current_dir(root)
            .args(["test", "--release", "-p", r.krate, "--test", r.target])
            .args(["--", "--ignored", "--nocapture", "--exact", r.test])
            .stderr(Stdio::inherit())
            .output()
            .with_context(|| format!("running cargo test for {}", r.test))?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        for line in stdout.lines().filter(|l| !l.trim().is_empty()) {
            println!("    {line}");
        }
        if !out.status.success() {
            failed.push(r.test);
        }
    }
    if failed.is_empty() {
        println!("\nperf: {} report(s) passed", REPORTS.len());
        Ok(())
    } else {
        Err(anyhow!("perf report(s) failed: {}", failed.join(", ")))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::REPORTS;

    /// The trees `grep -rn 'xtask perf' crates src-tauri xtask` reads.
    const SCANNED: &[&str] = &["crates", "src-tauri", "xtask"];

    fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            if path.is_dir() {
                if !matches!(name.to_str(), Some("target" | "node_modules" | ".git")) {
                    rust_files(&path, out);
                }
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    /// The package that owns `file`: the nearest `Cargo.toml`'s `name`.
    fn package_of(file: &Path) -> String {
        let mut dir = file.parent();
        while let Some(d) = dir {
            if let Ok(toml) = fs::read_to_string(d.join("Cargo.toml")) {
                let name = toml
                    .lines()
                    .find_map(|l| l.trim().strip_prefix("name = \""))
                    .and_then(|l| l.strip_suffix('"'));
                return name.expect("a package name").to_owned();
            }
            dir = d.parent();
        }
        panic!("{} has no Cargo.toml", file.display());
    }

    /// `(package, target, test)` for every `#[ignore = "…"]` naming
    /// `cargo xtask perf`; a test outside `tests/` reads as target `lib`,
    /// which no table row runs.
    fn ignored_perf_tests(root: &Path) -> BTreeSet<(String, String, String)> {
        let mut files = Vec::new();
        for dir in SCANNED {
            rust_files(&root.join(dir), &mut files);
        }
        let mut found = BTreeSet::new();
        for file in files {
            let src = fs::read_to_string(&file).expect("readable source");
            let lines: Vec<&str> = src.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                let l = line.trim();
                if !(l.starts_with("#[ignore") && l.contains("xtask perf")) {
                    continue;
                }
                let name = lines[i + 1..]
                    .iter()
                    .find_map(|l| {
                        let l = l.trim();
                        let l = l.strip_prefix("async ").unwrap_or(l);
                        l.strip_prefix("fn ")
                    })
                    .and_then(|l| l.split(['(', '<']).next())
                    .unwrap_or_else(|| panic!("{}:{} has no fn", file.display(), i + 1));
                let target = match file.parent().and_then(Path::file_name) {
                    Some(d) if d == "tests" => file.file_stem().expect("a stem"),
                    _ => "lib".as_ref(),
                };
                found.insert((
                    package_of(&file),
                    target.to_string_lossy().into_owned(),
                    name.to_owned(),
                ));
            }
        }
        found
    }

    #[test]
    fn the_table_names_exactly_the_ignored_perf_tests() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("the workspace root");
        let table: BTreeSet<_> = REPORTS
            .iter()
            .map(|r| (r.krate.to_owned(), r.target.to_owned(), r.test.to_owned()))
            .collect();
        let tree = ignored_perf_tests(root);
        assert!(!tree.is_empty(), "the scan found no ignored perf test");
        assert_eq!(
            table, tree,
            "REPORTS must name exactly the tree's ignored perf tests"
        );
    }
}
