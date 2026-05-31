//! TUI system — interactive terminal user interface.
//!
//! Feature-gated behind `tui`. Provides the full interactive dashboard,
//! channel watch, DM inbox, voice session, friend list, doctor, and
//! community info views.
//!
//! Entry point: `run(cli)` — called from `main.rs` when output mode is Tui.

pub mod action;
pub mod app;
pub mod components;
pub mod event;
pub mod focus;
pub mod keybinds;
pub mod navigator;
pub mod terminal;
pub mod theme;

use std::sync::Arc;

use crate::cli::Cli;
use crate::transport::DaemonClient;

/// TUI entry point.
///
/// Lifecycle:
/// 1. Load and validate config
/// 2. Connect to daemon via DaemonClient
/// 3. Request initial status to verify daemon is operational
/// 4. Load theme and keymap
/// 5. Create `Tui` (no transport subscription — events come via IPC Subscribe)
/// 6. Create `App` with daemon client, config, theme, keymap
/// 7. Run `App::run()` — the main event loop
/// 8. On exit: drop `Tui` (restores terminal), shutdown client
pub async fn run(cli: Cli) -> anyhow::Result<()> {
    let term = std::env::var("TERM").unwrap_or_default();
    if term == "dumb" {
        anyhow::bail!(
            "TUI requires an interactive terminal (TERM=dumb detected)\n\
             use one-shot CLI commands instead: rekindle status, rekindle doctor, etc.\n\
             or set --format json for machine-readable output"
        );
    }

    let config = crate::config::load(cli.config.as_deref())?;
    crate::config::validate(&config)?;

    let mut client = DaemonClient::connect().await?;

    // Verify daemon is operational. Retry with backoff because the daemon's
    // internal bus subscriber may not be connected yet (78ms race window
    // between server bind and subscriber handshake completion).
    let status = {
        let mut last_err = None;
        let mut result = None;
        for attempt in 1..=5u32 {
            match client
                .request_ok(rekindle_node::ipc::protocol::IpcRequest::Status)
                .await
            {
                Ok(v) => {
                    result = Some(v);
                    break;
                }
                Err(e) => {
                    tracing::debug!(attempt, error = %e, "daemon not ready, retrying");
                    last_err = Some(e);
                    tokio::time::sleep(std::time::Duration::from_millis(200 * u64::from(attempt)))
                        .await;
                }
            }
        }
        result
            .ok_or_else(|| last_err.unwrap_or_else(|| anyhow::anyhow!("daemon not responding")))?
    };
    tracing::info!(state = %status["state"], "daemon connected");

    // Subscribe to all events for real-time TUI rendering
    if let Err(e) = client.subscribe_all().await {
        tracing::warn!(error = %e, "event subscription failed — TUI will use polling only");
    }

    // Take the event receiver before wrapping in Arc
    let event_rx = client.take_event_receiver();
    let client = Arc::new(client);

    let theme_manager = theme::ThemeManager::load(&config.tui.theme)?;
    let keymap_store = keybinds::KeymapStore::load()?;

    let mut tui = terminal::Tui::new(&config.tui)
        .map_err(|e| anyhow::anyhow!("terminal initialization failed: {e}"))?;
    let mut application = app::App::new(Arc::clone(&client), config, theme_manager, keymap_store);

    let result = application.run(&mut tui, event_rx).await;

    drop(application);
    drop(tui);
    drop(client);

    result
}
