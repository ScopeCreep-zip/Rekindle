//! `cargo xtask check-supply-chain`: `cargo deny check` over the four
//! policy checks (advisories, bans, licenses, sources) that `deny.toml`
//! configures (docs/security/supply-chain-policy.md, plan SC1).
//!
//! It is part of `cargo xtask check`, so every gate run sees a new RustSec
//! advisory, a banned crate, an unlisted license or an unknown source the
//! day it appears, not only the scheduled `audit.yml` job. `cargo-deny` must
//! be installed (`cargo install cargo-deny --locked`,
//! docs/contributor/dependency-audit.md); a missing tool fails the check.
//!
//! `deny.toml` lets `libsqlite3-sys` alone wrap `openssl-sys`, for
//! SQLCipher's page cipher on the targets that build it with the vendored
//! OpenSSL. cargo-deny cannot scope a ban to targets, so the Apple half of
//! the ban is checked here: on Apple targets SQLCipher takes CommonCrypto
//! (plan SC1 slice 3) and `openssl-sys` must not be in the build graph.

use std::path::Path;
use std::process::Command;

use anyhow::{anyhow, Context, Result};

/// The `cargo deny` checks the policy requires, in its order.
const CHECKS: [&str; 4] = ["advisories", "bans", "licenses", "sources"];

/// The Apple targets whose build graph must hold no `openssl-sys`.
const APPLE_TARGETS: [&str; 3] = [
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "aarch64-apple-ios",
];

/// Runs `cargo deny --locked check` with every policy check at `root`.
pub(crate) fn check_supply_chain(root: &Path) -> Result<()> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let status = Command::new(&cargo)
        .args(["deny", "--locked", "check", "--hide-inclusion-graph"])
        .args(CHECKS)
        .current_dir(root)
        .status()
        .context("running `cargo deny` (install it: cargo install cargo-deny --locked)")?;
    if !status.success() {
        return Err(anyhow!(
            "cargo deny check {} failed ({status}); see its output above",
            CHECKS.join(" ")
        ));
    }
    for target in APPLE_TARGETS {
        check_no_openssl_on(&cargo, root, target)?;
    }
    Ok(())
}

/// Fails when `openssl-sys` is a normal or build dependency of the
/// workspace for `target` (`cargo tree` prints nothing when it is not).
fn check_no_openssl_on(cargo: &str, root: &Path, target: &str) -> Result<()> {
    let out = Command::new(cargo)
        .args(["tree", "--locked", "--workspace", "-e", "normal,build"])
        .args(["-i", "openssl-sys", "--target", target])
        .current_dir(root)
        .output()
        .context("running `cargo tree -i openssl-sys`")?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    let absent = if out.status.success() {
        out.stdout.is_empty()
    } else {
        stderr.contains("did not match any packages")
    };
    if absent {
        Ok(())
    } else {
        Err(anyhow!(
            "openssl-sys is in the {target} build graph; Apple builds take SQLCipher's \
             CommonCrypto provider (plan SC1):\n{}{stderr}",
            String::from_utf8_lossy(&out.stdout)
        ))
    }
}
