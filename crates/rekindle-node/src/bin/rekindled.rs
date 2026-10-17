//! `rekindled` — the Rekindle backend host. Every frontend (GUI, TUI, CLI)
//! is a client of this process over the IPC bus (ADR 0010).

// Byte-index string slicing panics inside a multi-byte character; cut with
// `rekindle_utils::text::{prefix, abbreviate}` instead (plan C2).
#![deny(clippy::string_slice)]

use std::io::Write;
use std::process::ExitCode;

use clap::Parser;

#[tokio::main]
async fn main() -> ExitCode {
    rekindle_utils::log_scrub::install_panic_hook();
    let args = rekindle_node::host::HostArgs::parse();
    let _log = match rekindle_node::host::init_tracing() {
        Ok(guard) => guard,
        Err(e) => return fail(&e),
    };
    tracing::debug!(?args, "rekindled starting");
    match run().await {
        Ok(reason) => ExitCode::from(reason.exit_code()),
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "rekindled failed");
            fail(&e)
        }
    }
}

/// Claim the data root, start the host over its own node, serve until asked
/// to stop, then stop gracefully (plan D3.1).
async fn run() -> anyhow::Result<rekindle_node::daemon::shutdown::ExitReason> {
    use rekindle_node::host::{self, Host, HostConfig, HostStores, TransportSource};
    let root = host::claim(rekindle_db::paths::DataRoot::resolve()?)?;
    let identities = host::identity::list(root.root()).map_err(|e| anyhow::anyhow!("{e}"))?;
    tracing::info!(count = identities.len(), "identity registry read");
    let host = Host::start(HostConfig {
        root,
        stores: HostStores::default(),
        transport: TransportSource::Start(host::user_transport_config()?),
        serve_bus: true,
    })
    .await?;
    host.run_until_stopped().await?;
    host.shutdown().await
}

/// Report a fatal error on stderr — the one channel a supervisor or the
/// launching `rekindle node start` sees — and exit non-zero: `EX_CONFIG`
/// for a fault a restart cannot fix, 1 otherwise.
fn fail(error: &anyhow::Error) -> ExitCode {
    // A closed stderr leaves nothing else to report to.
    let _ = writeln!(std::io::stderr(), "rekindled: {error:#}");
    if error
        .downcast_ref::<rekindle_node::host::ConfigFault>()
        .is_some()
    {
        ExitCode::from(rekindle_node::host::ConfigFault::EXIT_CODE)
    } else {
        ExitCode::FAILURE
    }
}
