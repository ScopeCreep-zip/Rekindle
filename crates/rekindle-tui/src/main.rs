//! `rekindle-tui` — the terminal frontend: dashboard, channel watch, DM
//! inbox, voice session, friend list, doctor and community info views, all
//! rendered from the daemon's event stream. A client of `rekindled` like
//! every frontend (ADR 0010).

#![forbid(unsafe_code)]
// Byte-index string slicing panics inside a multi-byte character (plan C2).
#![deny(clippy::string_slice)]

mod action;
mod app;
mod components;
mod event;
mod focus;
mod helpers;
mod keybinds;
mod navigator;
mod presence_fmt;
mod terminal;
mod theme;
mod views;

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;

/// `rekindle-tui`'s command line.
#[derive(Debug, Parser)]
#[command(name = "rekindle-tui", version, about = "Rekindle terminal UI")]
struct TuiArgs {
    /// Read this config file after every other layer.
    #[arg(long)]
    config: Option<PathBuf>,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    // color-eyre formats panics and error reports; panic output still goes
    // through the identifier scrubber like every log line.
    let (panic_hook, eyre_hook) = color_eyre::config::HookBuilder::default().into_hooks();
    if let Err(e) = eyre_hook.install() {
        return fail(&anyhow::anyhow!("color-eyre install failed: {e}"));
    }
    rekindle_utils::log_scrub::install_panic_hook_with(move |info| {
        panic_hook.panic_report(info).to_string()
    });
    let args = TuiArgs::parse();
    let _log = match rekindle_client::log::init("rekindle-tui.log") {
        Ok(guard) => guard,
        Err(e) => return fail(&e.into()),
    };
    match run(args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(&e),
    }
}

/// Report a fatal error on stderr (the terminal is restored by then).
fn fail(error: &anyhow::Error) -> ExitCode {
    // A closed stderr leaves nothing else to report to.
    let _ = writeln!(std::io::stderr(), "rekindle-tui: {error:#}");
    ExitCode::FAILURE
}

/// TUI entry point.
///
/// Lifecycle:
/// 1. Load and validate config
/// 2. Connect to the daemon, starting it if needed (`connect_or_spawn`)
/// 3. Request initial status
/// 4. Load theme and keymap
/// 5. Create `Tui` (no transport subscription — events come via IPC Subscribe)
/// 6. Create `App` with daemon client, config, theme, keymap
/// 7. Run `App::run()` — the main event loop
/// 8. On exit: drop `Tui` (restores terminal), shutdown client
async fn run(args: TuiArgs) -> anyhow::Result<()> {
    let term = std::env::var("TERM").unwrap_or_default();
    if term == "dumb" {
        anyhow::bail!(concat!(
            "TUI requires an interactive terminal (TERM=dumb detected)\n",
            "use one-shot commands instead: ",
            rekindle_client::cli_cmd!("status"),
            ", ",
            rekindle_client::cli_cmd!("status --doctor"),
            ", etc.\nor set --format json for machine-readable output"
        ));
    }

    let config = rekindle_client::config::load(args.config.as_deref())?;

    // Start the daemon if none is running (GnuPG-style on-demand start);
    // returns once it answers.
    let spawn = rekindle_client::spawn::SpawnOpts::beside_current_exe()?;
    let client = rekindle_client::spawn::connect_or_spawn(&spawn).await?;
    let status = client
        .request_ok(rekindle_ipc::protocol::IpcRequest::Status)
        .await?;
    tracing::info!(state = %status["state"], "daemon connected");

    // Every view renders from this event stream; without it the TUI would
    // show stale data, so a refused subscription is an error.
    client.subscribe_all().await?;

    // Take the event receiver before wrapping in Arc
    let event_rx = client.take_event_receiver();
    let client = Arc::new(client);

    let theme_manager = theme::ThemeManager::load(&config.tui.theme)?;
    let keymap_store = keybinds::KeymapStore::load()?;

    let mut tui = terminal::Tui::new(&config.tui)
        .map_err(|e| anyhow::anyhow!("terminal initialization failed: {e}"))?;
    let mut application = app::App::new(Arc::clone(&client), &config, theme_manager, keymap_store);

    let result = application.run(&mut tui, event_rx).await;

    drop(application);
    drop(tui);
    drop(client);

    result
}
