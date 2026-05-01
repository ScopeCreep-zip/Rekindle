//! Network, node lifecycle, and status commands.

mod peers;
mod route;
mod status;

use crate::cli::{NetworkCmd, NodeCmd, StatusArgs};
use crate::output::OutputMode;
use crate::transport::TransportHandle;

/// `rekindle status` — show node status overview.
pub async fn cmd_status(
    args: &StatusArgs,
    handle: &TransportHandle,
    session: &rekindle_transport::Session,
    mode: OutputMode,
) -> anyhow::Result<()> {
    // If --doctor flag is present, delegate to doctor
    if let Some(ref categories) = args.doctor {
        let doctor_args = crate::cli::DoctorArgs {
            categories: categories.clone(),
            output: args.output.clone().unwrap_or_else(|| "text".into()),
            exit_code: args.exit_code,
            quiet: args.quiet,
        };
        return crate::doctor::cmd_doctor(&doctor_args, handle, session, mode).await;
    }

    status::cmd_status(handle, session, args.watch, mode).await
}

/// Dispatch `rekindle node <subcommand>`.
pub fn dispatch_node(
    cmd: &NodeCmd,
    _handle: &TransportHandle,
    _mode: OutputMode,
) -> anyhow::Result<()> {
    match cmd {
        NodeCmd::Start { .. } => {
            // Node is already started by transport::acquire()
            crate::output::format::print_text("Node is running.")
        }
        NodeCmd::Stop => {
            crate::output::format::print_text("Node will stop on exit.")?;
            Ok(())
        }
        NodeCmd::Restart => {
            crate::output::format::print_text(
                "Restart is not supported in one-shot CLI mode.\n\
                 Stop and re-run the command.",
            )
        }
        NodeCmd::Attach => {
            crate::output::format::print_text("Node is attached (automatic on startup).")
        }
        NodeCmd::Detach => {
            crate::output::format::print_text(
                "Detach is not supported in one-shot CLI mode.\n\
                 The node detaches automatically on exit.",
            )
        }
    }
}

/// Dispatch `rekindle network <subcommand>`.
pub async fn dispatch_network(
    cmd: &NetworkCmd,
    handle: &TransportHandle,
    _session: &rekindle_transport::Session,
    mode: OutputMode,
) -> anyhow::Result<()> {
    match cmd {
        NetworkCmd::Status => status::cmd_network_status(handle, mode),
        NetworkCmd::Peers { format: fmt } => {
            let effective_mode = fmt
                .as_deref()
                .map_or(mode, |f| match f {
                    "json" => OutputMode::Json,
                    "jsonl" => OutputMode::Jsonl,
                    _ => mode,
                });
            peers::cmd_peers(handle, effective_mode)
        }
        NetworkCmd::Routes { refresh } => route::cmd_routes(handle, *refresh, mode).await,
        NetworkCmd::Config => status::cmd_network_config(handle, mode),
    }
}
