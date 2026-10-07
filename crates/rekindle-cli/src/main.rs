#![recursion_limit = "512"]
//! `rekindle`, the command-line frontend.
//!
//! A client of `rekindled` like every frontend (ADR 0010): each command
//! sends an `IpcRequest` over the Noise IK bus and renders the response.
//! The interactive terminal UI is the separate `rekindle-tui`.

#![forbid(unsafe_code)]
#![deny(clippy::print_stdout)]
// Byte-index string slicing panics inside a multi-byte character; cut with
// `rekindle_utils::text::{prefix, abbreviate}` instead (plan C2).
#![deny(clippy::string_slice)]

mod cli;
mod config;
mod error;
mod helpers;
mod output;
mod watch;

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

use clap::{CommandFactory, Parser};
use owo_colors::OwoColorize;

use cli::{Cli, Command};
use output::OutputMode;
use rekindle_client::spawn::{self, SpawnOpts, Started};
use rekindle_client::DaemonClient;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    rekindle_utils::log_scrub::install_panic_hook();
    let cli = Cli::parse();
    let _log = match rekindle_client::log::init("rekindle.log") {
        Ok(guard) => Some(guard),
        Err(e) => {
            output::format::eprint_line(&format!("error: {e}"));
            std::process::exit(1);
        }
    };

    let is_structured = matches!(cli.format.as_deref(), Some("json" | "jsonl"));
    output::format::set_quiet(cli.quiet || is_structured);
    output::color::set_no_color(cli.no_color);
    helpers::set_no_input(cli.no_input);

    let mode = OutputMode::detect(cli.format.as_deref(), cli.script);

    if let Err(e) = run(cli, mode).await {
        let code = error::exit_code(&e);
        // Scripts reading JSON get the error on the same stream they parse.
        if mode.is_structured()
            && output::format::print_structured(&error::to_json(&e), mode).is_ok()
        {
            std::process::exit(code);
        }
        if mode.use_color() {
            output::format::eprint_line(&format!("{}: {e:#}", "error".red().bold()));
        } else {
            output::format::eprint_line(&format!("error: {e:#}"));
        }
        if let Some(hint) = error::remediation(&e) {
            output::format::eprint_line(&format!("  {hint}"));
        }
        std::process::exit(code);
    }
}

async fn run(cli: Cli, mode: OutputMode) -> anyhow::Result<()> {
    let Some(command) = cli.command else {
        Cli::command().print_help()?;
        return Ok(());
    };
    match command {
        Command::Completions { shell } => {
            cli::print_completions(shell);
            Ok(())
        }
        Command::Config(cmd) => config::dispatch(&cmd, cli.config.as_deref(), mode),
        // `node start` launches `rekindled`; there is nothing to connect to yet.
        Command::Node(cli::NodeCmd::Start { foreground }) => node_start(foreground, mode).await,
        // Status reports a stopped daemon honestly rather than starting one.
        Command::Status(args) => match DaemonClient::connect().await {
            Ok(client) => {
                let result = network::cmd_status(&client, &args, mode).await;
                client.shutdown().await;
                result
            }
            Err(rekindle_client::ClientError::NotRunning { .. }) => {
                network::cmd_status_offline(mode)
            }
            Err(e) => Err(e.into()),
        },
        // Stopping needs no daemon started first.
        Command::Node(cli::NodeCmd::Stop) => {
            let client = DaemonClient::connect().await?;
            let result = dispatch_node(cli::NodeCmd::Stop, &client, mode).await;
            client.shutdown().await;
            result
        }
        command => {
            // Like gpg with gpg-agent, a command that needs the daemon
            // starts it.
            let client = spawn::connect_or_spawn(&SpawnOpts::beside_current_exe()?).await?;
            let result = dispatch_command(command, &client, mode).await;
            client.shutdown().await;
            result
        }
    }
}

/// `node start`: launch `rekindled`, detached or in this terminal.
async fn node_start(foreground: bool, mode: OutputMode) -> anyhow::Result<()> {
    let opts = SpawnOpts::beside_current_exe()?;
    if foreground {
        spawn::run_foreground(&opts.daemon_path)?;
        return Ok(());
    }
    let (state, pid) = match spawn::start_detached(&opts).await? {
        Started::AlreadyRunning => ("already running", None),
        Started::Spawned { pid } => ("started", Some(pid)),
    };
    if mode.is_structured() {
        output::format::print_structured(&serde_json::json!({ "daemon": state, "pid": pid }), mode)
    } else {
        match pid {
            Some(pid) => output::format::print_text(&format!("rekindled {state} (pid {pid})")),
            None => output::format::print_text(&format!("rekindled {state}")),
        }
    }
}

async fn dispatch_node(
    cmd: cli::NodeCmd,
    client: &DaemonClient,
    mode: OutputMode,
) -> anyhow::Result<()> {
    match cmd {
        cli::NodeCmd::Start { .. } => unreachable!("node start is handled before connecting"),
        cli::NodeCmd::Stop => {
            let value = client
                .request_ok(rekindle_ipc::protocol::IpcRequest::Shutdown)
                .await?;
            if mode.is_structured() {
                output::format::print_structured(&value, mode)
            } else {
                output::format::print_text("Daemon shutdown initiated.")
            }
        }
        cli::NodeCmd::Restart => Err(identity::unimplemented("node restart")),
        cli::NodeCmd::Attach => Err(identity::unimplemented("node attach")),
        cli::NodeCmd::Detach => Err(identity::unimplemented("node detach")),
    }
}

async fn dispatch_command(
    command: Command,
    client: &DaemonClient,
    mode: OutputMode,
) -> anyhow::Result<()> {
    match command {
        Command::Completions { .. } | Command::Config(_) | Command::Status(_) => {
            unreachable!("handled before connecting")
        }
        Command::Init(args) => identity::cmd_init(&args, client, mode).await,
        Command::Unlock(args) => identity::cmd_unlock(&args, client, mode).await,
        Command::Lock => identity::cmd_lock(client, mode).await,
        Command::Identity(cmd) => identity::dispatch(&cmd, client, mode).await,
        Command::Node(cmd) => dispatch_node(cmd, client, mode).await,
        Command::Network(cmd) => network::dispatch(&cmd, client, mode).await,
        Command::Friend(cmd) => friends::dispatch(&cmd, client, mode).await,
        Command::Dm(cmd) => dm::dispatch(&cmd, client, mode).await,
        Command::Community(cmd) => community::dispatch(&cmd, client, mode).await,
        Command::Role(cmd) => governance::dispatch_role(&cmd, client, mode).await,
        Command::Moderate(cmd) => governance::dispatch_moderate(&cmd, client, mode).await,
        Command::Channel(cmd) => channel::dispatch(&cmd, client, mode).await,
        Command::Voice(cmd) => voice::dispatch(&cmd, client, mode).await,
        Command::Key(cmd) => keys::dispatch(&cmd, client, mode).await,
        Command::Presence(cmd) => presence::dispatch(&cmd, client, mode).await,
        Command::Export(cmd) => identity::dispatch_export(&cmd, client, mode).await,
        Command::Import(_) => Err(identity::unimplemented("import")),
    }
}
