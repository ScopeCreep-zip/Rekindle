#![recursion_limit = "512"]
//! Entrypoint for the `rekindle` CLI binary.
//!
//! The CLI is an IPC client to the rekindle-node daemon. Every command
//! sends an `IpcRequest` over the Noise IK encrypted bus and renders
//! the `IpcResponse`. The CLI never touches `TransportNode`, `Session`,
//! or the OS keyring directly.

#![forbid(unsafe_code)]
#![deny(clippy::print_stdout)]

mod cli;
mod config;
mod error;
mod helpers;
mod output;
mod transport;

mod channel;
mod community;
mod dm;
mod friends;
mod governance;
mod identity;
mod keys;
mod network;
mod presence;
mod voice;

#[cfg(feature = "daemon")]
mod node_daemon;

#[cfg(feature = "tui")]
mod tui;
#[cfg(feature = "tui")]
mod views;

use clap::Parser;
use owo_colors::OwoColorize;

use cli::{Cli, Command};
use output::OutputMode;
use transport::DaemonClient;

#[allow(clippy::print_stderr)]
#[tokio::main(flavor = "current_thread")]
async fn main() {
    #[cfg(feature = "tui")]
    if let Err(e) = color_eyre::install() {
        eprintln!("warning: color-eyre install failed: {e}");
    }

    let _guard = helpers::init_tracing();
    let cli = Cli::parse();

    let is_structured = matches!(cli.format.as_deref(), Some("json" | "jsonl"));
    output::format::set_quiet(cli.quiet || is_structured);

    let is_tui_command = matches!(
        &cli.command,
        None | Some(
            Command::Channel(cli::ChannelCmd::Watch { .. })
                | Command::Dm(cli::DmCmd::Watch { .. })
                | Command::Voice(cli::VoiceCmd::Join { .. })
        )
    );

    let mode = OutputMode::detect(
        cli.format.as_deref(),
        is_tui_command,
        cli.no_color,
        cli.script,
    );

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
    if let Some(Command::Completions { shell }) = &cli.command {
        cli::print_completions(*shell);
        return Ok(());
    }

    let cfg = config::load(cli.config.as_deref())
        .map_err(|e| anyhow::anyhow!(error::CliError::Config(e.to_string())))?;
    config::validate(&cfg)
        .map_err(|e| anyhow::anyhow!(error::CliError::Validation(e.to_string())))?;

    match &cli.command {
        Some(Command::Config(cmd)) => return config::dispatch(cmd, &cfg, mode),
        None => {
            Cli::parse_from(["rekindle", "--help"]);
            unreachable!()
        }
        _ => {}
    }

    // `node start` runs the daemon in-process — no IPC client needed.
    #[cfg(feature = "daemon")]
    if let Some(Command::Node(cli::NodeCmd::Start { attach_timeout, .. })) = &cli.command {
        let timeout = *attach_timeout;
        return node_daemon::run_daemon(timeout).await;
    }
    #[cfg(not(feature = "daemon"))]
    if let Some(Command::Node(cli::NodeCmd::Start { .. })) = &cli.command {
        anyhow::bail!(
            "daemon support not compiled\n\
             rebuild with: cargo build --features daemon\n\
             or install the full package from your distribution"
        );
    }

    // Status is special: it must return local info even when the daemon is down.
    if let Some(Command::Status(ref args)) = cli.command {
        match DaemonClient::connect().await {
            Ok(client) => {
                let result = network::cmd_status(&client, args, mode).await;
                client.shutdown().await;
                return result;
            }
            Err(_) => {
                return network::cmd_status_offline(mode);
            }
        }
    }

    let client = DaemonClient::connect().await?;

    let result =
        dispatch_command(cli.command.expect("command required"), &client, &cfg, mode).await;

    client.shutdown().await;
    result
}

#[allow(clippy::too_many_lines)]
async fn dispatch_command(
    command: Command,
    client: &DaemonClient,
    cfg: &config::schema::Config,
    mode: OutputMode,
) -> anyhow::Result<()> {
    match command {
        Command::Completions { .. } | Command::Config(_) | Command::Status(_) => {
            unreachable!("handled before dispatch")
        }
        Command::Init(args) => identity::cmd_init(&args, client, mode).await,
        Command::Identity(cmd) => identity::dispatch(&cmd, client, mode).await,
        Command::Node(cmd) => match cmd {
            cli::NodeCmd::Start { .. } => unreachable!("handled before daemon connect"),
            cli::NodeCmd::Stop => {
                let value = client
                    .request_ok(rekindle_node::ipc::protocol::IpcRequest::Shutdown)
                    .await?;
                if mode.is_structured() {
                    output::format::print_structured(&value, mode)
                } else {
                    output::format::print_text("Daemon shutdown initiated.")
                }
            }
            cli::NodeCmd::Restart => output::format::print_text(
                "Restart: use 'rekindle node stop && rekindle node start'",
            ),
            cli::NodeCmd::Attach | cli::NodeCmd::Detach => {
                let value = client
                    .request_ok(rekindle_node::ipc::protocol::IpcRequest::NetworkStatus)
                    .await?;
                output::format::print_structured(&value, mode)
            }
        },
        Command::Network(cmd) => network::dispatch(&cmd, client, mode).await,
        Command::Friend(cmd) => friends::dispatch(&cmd, client, mode).await,
        Command::Dm(cmd) => dm::dispatch(&cmd, client, mode).await,
        Command::Community(cmd) => community::dispatch(&cmd, client, cfg, mode).await,
        Command::Role(cmd) => governance::dispatch_role(&cmd, client, mode).await,
        Command::Moderate(cmd) => governance::dispatch_moderate(&cmd, client, mode).await,
        Command::Channel(cmd) => channel::dispatch(&cmd, client, mode).await,
        Command::Voice(cmd) => voice::dispatch(&cmd, client, mode).await,
        Command::Key(cmd) => keys::dispatch(&cmd, client, mode).await,
        Command::Presence(cmd) => presence::dispatch(&cmd, client, mode).await,
        Command::Export(cmd) => identity::dispatch_export(&cmd, client, mode).await,
        Command::Import(_cmd) => {
            output::format::print_text("Import: use 'rekindle init' after placing the bundle file")
        }
    }
}
