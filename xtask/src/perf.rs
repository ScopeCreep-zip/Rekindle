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
pub(crate) const REPORTS: &[Report] = &[
    Report {
        krate: "rekindle-skin",
        target: "serve_budget",
        test: "every_builtin_serves_its_stylesheet_in_time",
        reason: "each built-in's stylesheet is served in 1.5 s, inside the client's 5 s timeout \
                 (the work itself is gated by operation counts in serve_tests.rs)",
    },
    Report {
        krate: "rekindle-governance",
        target: "perf_targets",
        test: "merge_10k_entries_completes_within_budget",
        reason: "a merge of 10,000 governance entries finishes in 10 ms \
                 (the work itself is gated by merge_work_grows_linearly_in_the_entries)",
    },
    Report {
        krate: "rekindle-governance",
        target: "perf_targets",
        test: "an_uncached_snapshot_of_10k_entries_answers_inside_the_app_call_window",
        reason: "merge_cut + stateRoot over 10,000 entries answers inside Veilid's 5 s app_call \
                 window (its length is gated exactly by the preimage-length test)",
    },
    Report {
        krate: "rekindle-governance-runtime",
        target: "walk_cpu_budget",
        test: "a_sixty_four_head_walk_stays_within_its_cpu_budget",
        reason: "a 64-head op walk finishes in 100 ms \
                 (verifications and walk steps are gated exactly beside it)",
    },
];

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
#[path = "perf_tests.rs"]
mod tests;
