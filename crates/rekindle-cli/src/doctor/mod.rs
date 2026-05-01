//! System health diagnostics — `rekindle doctor`.
//!
//! Runs check modules in categories (node, crypto, network, storage),
//! collects results, and formats output as text or JSON. Exit codes
//! match the contract: 10=all pass, 11=any fail, 12=warn only.

mod crypto;
mod network;
mod node;
mod storage;

use serde::Serialize;

use crate::cli::DoctorArgs;
use crate::output::format;
use crate::output::OutputMode;
use crate::transport::TransportHandle;

/// A single diagnostic check result.
#[derive(Debug, Clone, Serialize)]
pub struct Check {
    /// Dotted identifier, e.g., "node.running".
    pub id: String,
    /// Category: "node", "crypto", "network", "storage".
    pub category: &'static str,
    /// Pass, Warn, or Fail.
    pub status: Status,
    /// Human-readable value, e.g., "active", "12 MB / 128 MB".
    pub value: String,
    /// Remediation hint (shown on non-Pass). Empty for Pass.
    pub description: String,
}

/// Check result status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Pass,
    Warn,
    Fail,
}

/// `rekindle doctor` — run health checks and report.
pub async fn cmd_doctor(
    args: &DoctorArgs,
    handle: &TransportHandle,
    session: &rekindle_transport::Session,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let categories: Vec<&str> = if args.categories == "all" {
        vec!["node", "crypto", "network", "storage"]
    } else {
        args.categories.split(',').map(str::trim).collect()
    };

    let mut checks = Vec::new();

    for cat in &categories {
        match *cat {
            "node" => checks.extend(node::checks(handle)),
            "crypto" => checks.extend(crypto::checks(handle, session).await),
            "network" => checks.extend(network::checks(handle).await),
            "storage" => checks.extend(storage::checks(session).await),
            unknown => {
                anyhow::bail!(
                    "unknown doctor category: '{unknown}'\n\
                     valid categories: node, crypto, network, storage, all"
                );
            }
        }
    }

    // Output
    let effective_mode = match args.output.as_str() {
        "json" => OutputMode::Json,
        _ => mode,
    };

    format::print_doctor_checks(&checks, effective_mode, args.quiet)?;

    // Exit code
    if args.exit_code {
        let has_fail = checks.iter().any(|c| c.status == Status::Fail);
        let has_warn = checks.iter().any(|c| c.status == Status::Warn);

        let code = if has_fail {
            11
        } else if has_warn {
            12
        } else {
            10
        };

        std::process::exit(code);
    }

    Ok(())
}
