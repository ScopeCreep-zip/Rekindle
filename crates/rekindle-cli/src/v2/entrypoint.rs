//! Entrypoint for the `rekindle` CLI binary.
//!
//! The CLI is an IPC client to the rekindle-node daemon. Every command
//! sends an `IpcRequest` over the Noise IK encrypted bus and renders
//! the `IpcResponse`. The CLI never touches `TransportNode`, `Session`,
//! or the OS keyring directly.
//!
//! Multiple CLI instances, TUI instances, and GUI clients can connect
//! simultaneously — the IPC bus server handles concurrent connections
//! with independent Noise IK sessions and subscription filters.

use clap::Parser;
use owo_colors::OwoColorize;

use crate::v2::cli::{Cli, Command};
use crate::v2::error;
use crate::v2::output::format;
use crate::v2::output::OutputMode;
use crate::v2::transport::DaemonClient;

/// CLI entry point — called from the crate's actual main.rs.
///
/// Handles: tracing init, mode detection, config load, command dispatch,
/// streaming command interception, TUI launch, exit code mapping.
#[allow(clippy::print_stderr)]
pub async fn run() {
    #[cfg(feature = "tui")]
    if let Err(e) = color_eyre::install() {
        eprintln!("warning: color-eyre install failed: {e}");
    }

    let cli = Cli::parse();

    // `node start` runs the daemon which constructs its own reload-capable
    // tracing subscriber. CLI must NOT initialize tracing before delegating.
    #[cfg(feature = "daemon")]
    if matches!(&cli.command, Some(Command::Node(crate::v2::cli::NodeCmd::Start { .. }))) {
        let timeout = match &cli.command {
            Some(Command::Node(crate::v2::cli::NodeCmd::Start { attach_timeout, .. })) => *attach_timeout,
            _ => unreachable!(),
        };
        if let Err(e) = crate::v2::commands::node_daemon::run_daemon(timeout).await {
            eprintln!("error: {e:#}");
            std::process::exit(1);
        }
        return;
    }

    // All other commands: CLI owns tracing
    let _guard = crate::v2::helpers::init_tracing();

    let is_structured = matches!(cli.format.as_deref(), Some("json" | "jsonl"));
    format::set_quiet(cli.quiet || is_structured);

    let is_tui_command = matches!(
        &cli.command,
        None
            | Some(
                Command::Channel(crate::v2::cli::ChannelCmd::Watch { .. })
                    | Command::Dm(crate::v2::cli::DmCmd::Watch { .. })
                    | Command::Voice(crate::v2::cli::VoiceCmd::Join { .. })
            )
    );

    let mode = OutputMode::detect(cli.format.as_deref(), is_tui_command, cli.no_color, cli.script);

    let result = match mode {
        #[cfg(feature = "tui")]
        OutputMode::Tui => crate::v2::tui::run(cli).await,
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
    if let Some(Command::Completions { shell }) = &cli.command {
        crate::v2::cli::print_completions(*shell);
        return Ok(());
    }

    let cfg = crate::v2::config::load(cli.config.as_deref())
        .map_err(|e| anyhow::anyhow!(error::CliError::Config(e.to_string())))?;
    crate::v2::config::validate(&cfg)
        .map_err(|e| anyhow::anyhow!(error::CliError::Validation(e.to_string())))?;

    match &cli.command {
        Some(Command::Config(cmd)) => return crate::v2::config::dispatch(cmd, &cfg, mode),
        None => { Cli::parse_from(["rekindle", "--help"]); unreachable!() }
        _ => {}
    }

    // Search, grep, and patch-apply are local filesystem operations — no daemon needed.
    if let Some(Command::Search { query, limit }) = &cli.command {
        return crate::v2::commands::search::cmd_search(query, *limit, mode);
    }
    if let Some(Command::Grep { query, regex, limit, before, after }) = &cli.command {
        return crate::v2::commands::search::cmd_grep(query, *regex, *limit, *before, *after, mode);
    }
    if let Some(Command::PatchApply { path, check }) = &cli.command {
        return crate::v2::commands::patch::cmd_patch_apply(path, *check, mode);
    }
    // `rekindle patch` without --send flags also works without daemon
    if let Some(Command::Patch { files, staged, channel_community, channel_name: _, dm_peer, message }) = &cli.command {
        if channel_community.is_none() && dm_peer.is_none() {
            // Local-only: generate and print to stdout
            return crate::v2::commands::patch::cmd_patch(
                files, *staged, None, None, None, message.as_deref(), None, mode,
            ).await;
        }
    }

    // `node start` is handled before tracing init in run() — never reaches here.
    // All other node subcommands (stop/restart/attach/detach) go through IPC.

    // Status is special: returns local info even when daemon is down.
    if let Some(Command::Status(ref args)) = cli.command {
        match DaemonClient::connect().await {
            Ok(client) => {
                if args.watch {
                    let result = watch_status_loop(&client, mode).await;
                    client.shutdown().await;
                    return result;
                }
                let result = crate::v2::commands::network::cmd_status(&client, args, mode).await;
                client.shutdown().await;
                return result;
            }
            Err(_) => {
                return crate::v2::commands::network::cmd_status_offline(mode);
            }
        }
    }

    let mut client = DaemonClient::connect().await?;
    let command = cli.command.expect("command required");

    // Streaming commands need the event receiver before dispatch.
    let result = match &command {
        Command::Dm(crate::v2::cli::DmCmd::Watch { friend }) if !matches!(mode, OutputMode::Tui) => {
            let mut event_rx = client.take_event_receiver()
                .ok_or_else(|| anyhow::anyhow!("event receiver unavailable"))?;
            crate::v2::commands::dm::watch_streaming(&client, &mut event_rx, friend.as_deref(), mode).await
        }
        Command::Channel(crate::v2::cli::ChannelCmd::Watch { community, channel, .. }) if !matches!(mode, OutputMode::Tui) => {
            let mut event_rx = client.take_event_receiver()
                .ok_or_else(|| anyhow::anyhow!("event receiver unavailable"))?;
            crate::v2::commands::channel::watch_streaming(&client, &mut event_rx, community, channel, mode).await
        }
        Command::Presence(crate::v2::cli::PresenceCmd::Watch { community }) => {
            let mut event_rx = client.take_event_receiver()
                .ok_or_else(|| anyhow::anyhow!("event receiver unavailable"))?;
            crate::v2::commands::presence::watch_streaming(&client, &mut event_rx, community.as_deref(), mode).await
        }
        _ => dispatch_command(command, &client, &cfg, mode).await,
    };

    client.shutdown().await;
    result
}

/// Continuous status refresh every 2 seconds.
#[allow(clippy::print_stderr)]
async fn watch_status_loop(client: &DaemonClient, mode: OutputMode) -> anyhow::Result<()> {
    use rekindle_node::ipc::protocol::IpcRequest;
    loop {
        if !mode.is_structured() {
            eprint!("\x1b[2J\x1b[H");
        }
        let value = client.request_ok(IpcRequest::Status).await?;
        let snapshot: rekindle_types::display::StatusSnapshot = serde_json::from_value(value)?;
        crate::v2::commands::network::print_status_compact(&snapshot, mode)?;
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

async fn dispatch_command(
    command: Command,
    client: &DaemonClient,
    _cfg: &crate::v2::config::schema::Config,
    mode: OutputMode,
) -> anyhow::Result<()> {
    match command {
        Command::Completions { .. } | Command::Config(_) | Command::Status(_) => {
            unreachable!("handled before dispatch")
        }
        Command::Init(args) => crate::v2::commands::identity::cmd_init(&args, client, mode).await,
        Command::Identity(cmd) => crate::v2::commands::identity::dispatch(&cmd, client, mode).await,
        Command::Node(cmd) => match cmd {
            crate::v2::cli::NodeCmd::Start { .. } => unreachable!("handled before daemon connect"),
            crate::v2::cli::NodeCmd::Stop => {
                let value = client.request_ok(rekindle_node::ipc::protocol::IpcRequest::Shutdown).await?;
                if mode.is_structured() { format::print_structured(&value, mode) }
                else { format::print_text("Daemon shutdown initiated.") }
            }
            crate::v2::cli::NodeCmd::Restart => {
                format::print_text("Restart: use 'rekindle node stop && rekindle node start'")
            }
            crate::v2::cli::NodeCmd::Attach | crate::v2::cli::NodeCmd::Detach => {
                let value = client.request_ok(rekindle_node::ipc::protocol::IpcRequest::NetworkStatus).await?;
                format::print_structured(&value, mode)
            }
        },
        Command::Network(cmd) => crate::v2::commands::network::dispatch(&cmd, client, mode).await,
        Command::Friend(cmd) => crate::v2::commands::friends::dispatch(&cmd, client, mode).await,
        Command::Dm(cmd) => crate::v2::commands::dm::dispatch(&cmd, client, mode).await,
        Command::Community(cmd) => crate::v2::commands::community::dispatch(&cmd, client, mode).await,
        Command::Role(cmd) => crate::v2::commands::governance::dispatch_role(&cmd, client, mode).await,
        Command::Moderate(cmd) => crate::v2::commands::governance::dispatch_moderate(&cmd, client, mode).await,
        Command::Channel(cmd) => crate::v2::commands::channel::dispatch(&cmd, client, mode).await,
        Command::Voice(cmd) => crate::v2::commands::voice::dispatch(&cmd, client, mode).await,
        Command::Key(cmd) => crate::v2::commands::keys::dispatch(&cmd, client, mode).await,
        Command::Presence(cmd) => crate::v2::commands::presence::dispatch(&cmd, client, mode).await,
        Command::Social(cmd) => crate::v2::commands::social::dispatch(&cmd, client, mode).await,
        Command::System(cmd) => crate::v2::commands::system::dispatch(&cmd, client, mode).await,
        Command::Export(cmd) => crate::v2::commands::identity::dispatch_export(&cmd, client, mode).await,
        Command::Import(_cmd) => {
            format::print_text("Import: use 'rekindle init' after placing the bundle file")
        }
        // Patch with --send (needs daemon for channel/DM send)
        Command::Patch { files, staged, channel_community, channel_name, dm_peer, message } => {
            crate::v2::commands::patch::cmd_patch(
                &files, staged,
                channel_community.as_deref(), channel_name.as_deref(),
                dm_peer.as_deref(), message.as_deref(),
                Some(client), mode,
            ).await
        }
        // PatchApply and Search/Grep are handled before daemon connect — never reach here.
        Command::PatchApply { .. } | Command::Search { .. } | Command::Grep { .. } => {
            unreachable!("handled before daemon connect")
        }
    }
}
