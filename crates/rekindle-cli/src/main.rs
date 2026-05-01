//! Entrypoint for the `rekindle` CLI binary.
//!
//! Responsibilities:
//! - Parse CLI arguments via clap
//! - Detect output mode (Tui/Text/Json/Jsonl) — single call, single decision point
//! - Initialize tracing to file via tracing-appender (never stdout)
//! - Dispatch to cli_run() for all non-TUI commands
//! - Format errors with what + why + remediation pattern
//! - Exit with correct exit code from the error contract

#![forbid(unsafe_code)]
#![deny(clippy::print_stdout)]

mod cli;
mod config;
mod error;
mod helpers;
mod output;
mod transport;

// Command modules
mod channel;
mod community;
mod dm;
mod doctor;
mod friends;
mod identity;
mod keys;
mod network;
mod presence;
mod voice;

// TUI (feature-gated)
#[cfg(feature = "tui")]
mod tui;
#[cfg(feature = "tui")]
mod views;

use clap::Parser;
use owo_colors::OwoColorize;

use cli::{Cli, Command};
use output::OutputMode;

#[allow(clippy::print_stderr)]
#[tokio::main(flavor = "current_thread")]
async fn main() {
    // Install color-eyre BEFORE ratatui init.
    // ratatui wraps the current panic hook, so color-eyre must be installed first.
    #[cfg(feature = "tui")]
    if let Err(e) = color_eyre::install() {
        eprintln!("warning: color-eyre install failed: {e}");
    }

    // Tracing to file — never stdout (would corrupt TUI buffer)
    let _guard = helpers::init_tracing();

    let cli = Cli::parse();

    let is_tui_command = matches!(
        &cli.command,
        None
            | Some(
                Command::Channel(cli::ChannelCmd::Watch { .. })
                    | Command::Dm(cli::DmCmd::Watch { .. })
                    | Command::Voice(cli::VoiceCmd::Join { .. })
            )
    );

    let mode = OutputMode::detect(cli.format.as_deref(), is_tui_command, cli.no_color);

    let result = match mode {
        #[cfg(feature = "tui")]
        OutputMode::Tui => tui::run(cli).await,
        _ => cli_run(cli, mode).await,
    };

    if let Err(e) = result {
        let code = error::exit_code(&e);
        if mode.use_color() {
            eprintln!("{}: {e:#}", "error".red().bold());
        } else {
            eprintln!("error: {e:#}");
        }
        if let Some(hint) = error::remediation(&e) {
            eprintln!("  {hint}");
        }
        std::process::exit(code);
    }
}

async fn cli_run(cli: Cli, mode: OutputMode) -> anyhow::Result<()> {
    // Commands that don't need config or transport
    if let Some(Command::Completions { shell }) = &cli.command {
        cli::print_completions(*shell);
        return Ok(());
    }

    // Load and validate config
    let cfg = config::load(cli.config.as_deref())
        .map_err(|e| anyhow::anyhow!(error::CliError::Config(e.to_string())))?;
    config::validate(&cfg)
        .map_err(|e| anyhow::anyhow!(error::CliError::Validation(e.to_string())))?;

    // Commands that need config but not transport
    match &cli.command {
        Some(Command::Config(cmd)) => return config::dispatch(cmd, &cfg, mode),
        None => {
            // Bare invocation without TUI feature — print help
            Cli::parse_from(["rekindle", "--help"]);
            unreachable!()
        }
        _ => {}
    }

    // Load session (if identity exists)
    let session_path = helpers::session_path()?;
    let session = rekindle_transport::Session::load(&session_path)?;

    // Commands that need session but check initialization
    if let Some(Command::Init(args)) = &cli.command {
        return identity::cmd_init(args, &cfg, session.as_ref(), mode).await;
    }

    // Everything below requires an initialized session
    let session = session.ok_or_else(|| {
        error::CliError::NotInitialized("no identity found — run: rekindle init".into())
    })?;

    // Acquire transport
    let handle = transport::acquire(&cfg, &cli).await?;

    // Dispatch command
    let result = dispatch_command(
        cli.command.expect("command required"),
        &handle,
        &cfg,
        &session,
        mode,
    )
    .await;

    // Shutdown transport if we own it
    handle.shutdown_if_owned().await?;

    result
}

async fn dispatch_command(
    command: Command,
    handle: &transport::TransportHandle,
    cfg: &config::schema::Config,
    session: &rekindle_transport::Session,
    mode: OutputMode,
) -> anyhow::Result<()> {
    match command {
        // Init is handled above (before transport acquisition)
        Command::Init(_) | Command::Completions { .. } | Command::Config(_) => unreachable!(),

        Command::Status(args) => network::cmd_status(&args, handle, session, mode).await,
        Command::Identity(cmd) => identity::dispatch(&cmd, handle, session, cfg, mode).await,
        Command::Node(cmd) => network::dispatch_node(&cmd, handle, mode),
        Command::Network(cmd) => network::dispatch_network(&cmd, handle, session, mode).await,
        Command::Friend(cmd) => friends::dispatch(&cmd, handle, session, mode).await,
        Command::Dm(cmd) => dm::dispatch(&cmd, handle, session, mode).await,
        Command::Community(cmd) => community::dispatch(&cmd, handle, session, cfg, mode).await,
        Command::Role(cmd) => community::dispatch_role(&cmd, handle, session, mode).await,
        Command::Moderate(cmd) => community::dispatch_moderate(&cmd, handle, session, mode).await,
        Command::Channel(cmd) => channel::dispatch(&cmd, handle, session, mode).await,
        Command::Voice(cmd) => voice::dispatch(&cmd, handle, session, mode).await,
        Command::Key(cmd) => keys::dispatch(&cmd, handle, session, mode).await,
        Command::Presence(cmd) => presence::dispatch(&cmd, handle, session, mode).await,
        Command::Doctor(args) => doctor::cmd_doctor(&args, handle, session, mode).await,
        Command::Export(cmd) => identity::dispatch_export(&cmd, session, mode).await,
        Command::Import(cmd) => identity::dispatch_import(&cmd, cfg, mode),
    }
}
