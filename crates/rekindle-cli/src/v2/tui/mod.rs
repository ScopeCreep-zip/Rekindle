//! TUI system — interactive terminal user interface.
//!
//! Entry point: `run(cli)` — called from entrypoint.rs when output mode is Tui.

pub mod components;
pub mod data_requirements;
pub mod effects;
pub mod event;
pub mod events;
pub mod focus;
pub mod idle;
pub mod keybinds;
pub mod machine;
pub mod palette;
pub mod process;
pub mod reconnect;
pub mod state;
pub mod terminal;
pub mod theme;
pub mod widgets;

use std::sync::Arc;

use crate::v2::cli::Cli;
use crate::v2::prelude::{DaemonClient, DaemonRequest, LifecycleRequest};

pub async fn run(cli: Cli) -> anyhow::Result<()> {
    let term = std::env::var("TERM").unwrap_or_default();
    if term == "dumb" {
        anyhow::bail!(
            "TUI requires an interactive terminal (TERM=dumb detected)\n\
             use one-shot CLI commands instead: rekindle status, rekindle doctor, etc.\n\
             or set --format json for machine-readable output"
        );
    }

    let config = crate::v2::config::load(cli.config.as_deref())?;
    crate::v2::config::validate(&config)?;

    let mut client = DaemonClient::connect().await?;

    let status = {
        let mut last_err = None;
        let mut result = None;
        for attempt in 1..=5u32 {
            match client.request_ok(DaemonRequest::Lifecycle(LifecycleRequest::Status)).await {
                Ok(v) => { result = Some(v); break; }
                Err(e) => {
                    tracing::debug!(attempt, error = %e, "daemon not ready, retrying");
                    last_err = Some(e);
                    tokio::time::sleep(std::time::Duration::from_millis(200 * u64::from(attempt))).await;
                }
            }
        }
        result.ok_or_else(|| last_err.unwrap_or_else(|| anyhow::anyhow!("daemon not responding")))?
    };

    tracing::info!(state = %status.get("state").and_then(|v| v.as_str()).unwrap_or("?"), "daemon connected");

    let event_rx = client.take_event_receiver()
        .ok_or_else(|| anyhow::anyhow!("event receiver unavailable"))?;
    let client = Arc::new(client);

    let tui = terminal::Tui::new(&config.tui)
        .map_err(|e| anyhow::anyhow!("terminal initialization failed: {e}"))?;

    machine::run(tui, client, event_rx, &config.tui).await
}
