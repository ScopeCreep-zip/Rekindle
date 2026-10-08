//! Identity commands: init, show, export, rotate, destroy.

use rekindle_ipc::protocol::{IpcRequest, DESTROY_CONFIRMATION, WIPE_CONFIRMATION};

use crate::cli::{ExportCmd, IdentityCmd, InitArgs, UnlockArgs};
use crate::error::{cmd, CliError};
use crate::helpers;
use crate::output::format;
use crate::output::OutputMode;
use rekindle_client::DaemonClient;

pub async fn cmd_init(
    args: &InitArgs,
    client: &DaemonClient,
    mode: OutputMode,
) -> anyhow::Result<()> {
    if args.wipe_all_data {
        let Some(confirmation) = helpers::confirm_severe(
            "This will delete ALL local Rekindle data.",
            WIPE_CONFIRMATION,
            args.confirm.as_deref(),
        )?
        else {
            return format::print_text("Cancelled.");
        };
        let value = client
            .request_ok(IpcRequest::IdentityWipe { confirmation })
            .await?;
        return format::print_structured(&value, mode);
    }

    let display_name = helpers::resolve_display_name(args.display_name.as_deref())?;
    let passphrase = helpers::read_passphrase(
        "Passphrase for this identity",
        args.passphrase_file.as_deref(),
    )?;

    // Check if identity already exists
    let existing = client.request_ok(IpcRequest::IdentityShow).await;
    if existing.is_ok() {
        format::step_skip("Identity already exists")?;
        format::step_header(1, 2, "Unlocking daemon")?;
        unlock(client, &passphrase).await?;
        format::step_done("daemon operational")?;
        let session_file = helpers::session_path()?;
        tracing::info!(path = %session_file.display(), "session state path");
        return format::print_text("Identity already initialized. Daemon unlocked.");
    }

    format::step_header(1, 2, "Creating identity via daemon")?;
    let value = client
        .request_ok(IpcRequest::IdentityCreate {
            display_name: display_name.clone(),
        })
        .await?;
    format::step_done("identity created")?;

    format::step_header(2, 2, "Unlocking daemon")?;
    unlock(client, &passphrase).await?;
    format::step_done("daemon operational")?;

    if mode.is_structured() {
        format::print_structured(&value, mode)
    } else {
        format::print_text("\nIdentity created successfully.")?;
        if let Some(pk) = value.get("public_key").and_then(|v| v.as_str()) {
            format::print_text(&format!("  Public key: {pk}"))?;
        }
        format::print_text(&format!("  Display name: {display_name}"))?;
        format::print_text("\nNext steps:")?;
        format::print_text(concat!(
            "  ",
            cmd!("status"),
            "              — check node health"
        ))?;
        format::print_text(concat!(
            "  ",
            cmd!("community create"),
            "    — create a community"
        ))?;
        format::print_text(concat!(
            "  ",
            cmd!("friend add"),
            "          — add a friend"
        ))
    }
}

/// `unlock`: read the passphrase and unlock the daemon. The daemon checks
/// it once the vault holds the identity (plan D1).
pub async fn cmd_unlock(
    args: &UnlockArgs,
    client: &DaemonClient,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let passphrase = helpers::read_passphrase("Passphrase", args.passphrase_file.as_deref())?;
    let value = unlock(client, &passphrase).await?;
    format::print_structured(&value, mode)
}

/// `lock`: take the daemon offline and drop the identity's keys.
pub async fn cmd_lock(client: &DaemonClient, mode: OutputMode) -> anyhow::Result<()> {
    let value = client.request_ok(IpcRequest::Lock).await?;
    format::print_structured(&value, mode)
}

async fn unlock(
    client: &DaemonClient,
    passphrase: &zeroize::Zeroizing<String>,
) -> anyhow::Result<serde_json::Value> {
    Ok(client
        .request_ok(IpcRequest::Unlock {
            passphrase: passphrase.as_str().to_owned(),
        })
        .await?)
}

pub async fn dispatch(
    cmd: &IdentityCmd,
    client: &DaemonClient,
    mode: OutputMode,
) -> anyhow::Result<()> {
    match cmd {
        IdentityCmd::Show { .. } => {
            let value = client.request_ok(IpcRequest::IdentityShow).await?;
            format::print_structured(&value, mode)
        }
        IdentityCmd::Rotate { force } => {
            if !helpers::confirm_unless_forced(
                *force,
                "Rotate identity keypair? All peers will need to re-verify.",
            )? {
                return format::print_text("Cancelled.");
            }
            let value = client.request_ok(IpcRequest::IdentityRotate).await?;
            format::print_structured(&value, mode)
        }
        IdentityCmd::Destroy { confirm } => {
            let Some(confirmation) = helpers::confirm_severe(
                "This will permanently destroy your identity.",
                DESTROY_CONFIRMATION,
                confirm.as_deref(),
            )?
            else {
                return format::print_text("Cancelled.");
            };
            let value = client
                .request_ok(IpcRequest::IdentityDestroy { confirmation })
                .await?;
            format::print_structured(&value, mode)
        }
        IdentityCmd::Export { path, passphrase } => {
            // A passphrase-sealed export needs the vault's export format
            // (plan D1); writing plaintext under a passphrase would only
            // look protected.
            if *passphrase {
                return Err(unimplemented("passphrase-protected identity export"));
            }
            let value = client.request_ok(IpcRequest::IdentityExport).await?;
            std::fs::write(path, serde_json::to_string_pretty(&value)?)?;
            format::print_text(&format!("Exported to {}", path.display()))
        }
        IdentityCmd::Import { .. } => Err(unimplemented("identity import")),
    }
}

pub async fn dispatch_export(
    cmd: &ExportCmd,
    client: &DaemonClient,
    _mode: OutputMode,
) -> anyhow::Result<()> {
    match cmd {
        ExportCmd::Identity { path } => {
            let value = client.request_ok(IpcRequest::IdentityExport).await?;
            let json = serde_json::to_string_pretty(&value)?;
            std::fs::write(path, &json)?;
            format::print_text(&format!("Exported to {}", path.display()))
        }
        ExportCmd::Friends { .. } => Err(unimplemented("export friends")),
        ExportCmd::Communities { .. } => Err(unimplemented("export communities")),
    }
}

/// A command that exists but does nothing yet: exit `EX_UNAVAILABLE` (69)
/// rather than pretend to succeed.
pub fn unimplemented(what: &str) -> anyhow::Error {
    anyhow::anyhow!(CliError::Unimplemented(what.to_owned()))
}
