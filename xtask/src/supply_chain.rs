//! `cargo xtask check-supply-chain`: `cargo deny check` over the four
//! policy checks (advisories, bans, licenses, sources) that `deny.toml`
//! configures (docs/security/supply-chain-policy.md, plan SC1).
//!
//! It is part of `cargo xtask check`, so every gate run sees a new RustSec
//! advisory, a banned crate, an unlisted license or an unknown source the
//! day it appears, not only the scheduled `audit.yml` job. `cargo-deny` must
//! be installed (`cargo install cargo-deny --locked`,
//! docs/contributor/dependency-audit.md); a missing tool fails the check.

use std::path::Path;
use std::process::Command;

use anyhow::{anyhow, Context, Result};

/// The `cargo deny` checks the policy requires, in its order.
const CHECKS: [&str; 4] = ["advisories", "bans", "licenses", "sources"];

/// Runs `cargo deny --locked check` with every policy check at `root`.
pub(crate) fn check_supply_chain(root: &Path) -> Result<()> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let status = Command::new(cargo)
        .args(["deny", "--locked", "check", "--hide-inclusion-graph"])
        .args(CHECKS)
        .current_dir(root)
        .status()
        .context("running `cargo deny` (install it: cargo install cargo-deny --locked)")?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!(
            "cargo deny check {} failed ({status}); see its output above",
            CHECKS.join(" ")
        ))
    }
}
