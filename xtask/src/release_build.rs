//! `check-release`: the workspace compiles under the release profile.
//!
//! Every other gate builds the debug profile, where `debug_assertions` is
//! on. An item gated `#[cfg(debug_assertions)]` that a production path
//! still names (a stray gate left on `services::call_runtime` once its
//! neighbour was deleted, REL1) compiles in every debug gate and breaks
//! only the shipped build. This gate runs `cargo check --release` over
//! every workspace member's library and binaries, so such a gap fails here.
//! Test targets stay out: unit tests of debug-only helpers (the
//! `REKINDLE_LOG_SCRUB_KEY` parser in `rekindle_utils::log_scrub`, the
//! `REKINDLE_ROUTE_BUDGET_CAPACITY` parser in the node's `route_budget`)
//! name items that exist only under `debug_assertions`, and tests never
//! ship.
//!
//! The WebDriver test driver is the one member left out: it refuses the
//! release profile by design (`compile_error!`, see
//! `check-webdriver-release`), and that gate proves no release build
//! reaches it.

use std::path::Path;
use std::process::Command;

use anyhow::{anyhow, Context, Result};

/// The `cargo` arguments of the release check.
fn release_check_args() -> Vec<&'static str> {
    vec![
        "check",
        "--release",
        "--workspace",
        "--exclude",
        crate::webdriver_release::CRATE,
    ]
}

pub(crate) fn check_release(root: &Path) -> Result<()> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let args = release_check_args();
    let status = Command::new(cargo)
        .args(&args)
        .current_dir(root)
        .status()
        .context("running cargo check --release")?;
    if status.success() {
        return Ok(());
    }
    Err(anyhow!(
        "`cargo {}` failed: a release build does not compile",
        args.join(" ")
    ))
}

#[cfg(test)]
#[path = "release_build_tests.rs"]
mod tests;
