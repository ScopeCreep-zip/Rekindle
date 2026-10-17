//! Identity commands: init, unlock, lock, logout, and `identity`'s list,
//! show, export, import, rotate and delete.

use std::path::Path;

use base64::Engine as _;
use rekindle_ipc::protocol::IpcRequest;

use crate::cli::{ExportCmd, IdentityCmd, ImportCmd, InitArgs, UnlockArgs};
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
        // Protected: the host asks for a passphrase (plan D3.3).
        helpers::set_passphrase_file(args.passphrase_file.clone());
        let value = helpers::send(client, IpcRequest::IdentityWipe).await?;
        return format::print_structured(&value, mode);
    }

    let display_name = helpers::resolve_display_name(args.display_name.as_deref())?;
    let passphrase = helpers::read_passphrase(
        "Passphrase for this identity",
        args.passphrase_file.as_deref(),
    )?;

    format::step_header(1, 2, "Creating identity via daemon")?;
    let value = helpers::send(
        client,
        IpcRequest::IdentityCreate {
            display_name: display_name.clone(),
            passphrase: passphrase.as_str().to_owned(),
        },
    )
    .await?;
    format::step_done("identity created")?;
    let public_key = value
        .get("public_key")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("the daemon did not name the new identity"))?
        .to_owned();

    format::step_header(2, 2, "Unlocking daemon")?;
    unlock(client, &public_key, &passphrase).await?;
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

/// `unlock`: pick the identity (`--identity`, or the only one on the
/// device), read its passphrase and unlock the daemon with it.
pub async fn cmd_unlock(
    args: &UnlockArgs,
    client: &DaemonClient,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let identity = match &args.identity {
        Some(identity) => identity.clone(),
        None => only_identity(client).await?,
    };
    let passphrase = helpers::read_passphrase("Passphrase", args.passphrase_file.as_deref())?;
    let value = unlock(client, &identity, &passphrase).await?;
    format::print_structured(&value, mode)
}

/// The device's one identity, or an error that says what to do.
async fn only_identity(client: &DaemonClient) -> anyhow::Result<String> {
    let value = helpers::send(client, IpcRequest::IdentityList).await?;
    let keys: Vec<String> = value
        .get("identities")
        .and_then(|v| v.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|e| e.get("publicKey").and_then(|k| k.as_str()))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    match keys.as_slice() {
        [only] => Ok(only.clone()),
        [] => Err(anyhow::anyhow!(concat!(
            "no identity on this device — create one with: ",
            cmd!("init")
        ))),
        _ => Err(anyhow::anyhow!(concat!(
            "this device holds several identities — name one with --identity (see: ",
            cmd!("identity list"),
            ")"
        ))),
    }
}

/// `lock`: take the daemon offline and drop the identity's keys.
pub async fn cmd_lock(client: &DaemonClient, mode: OutputMode) -> anyhow::Result<()> {
    let value = helpers::send(client, IpcRequest::Lock).await?;
    format::print_structured(&value, mode)
}

/// `logout`: go offline and close the identity.
pub async fn cmd_logout(client: &DaemonClient, mode: OutputMode) -> anyhow::Result<()> {
    let value = helpers::send(client, IpcRequest::Logout).await?;
    format::print_structured(&value, mode)
}

async fn unlock(
    client: &DaemonClient,
    identity: &str,
    passphrase: &zeroize::Zeroizing<String>,
) -> anyhow::Result<serde_json::Value> {
    helpers::send(
        client,
        IpcRequest::Unlock {
            identity: identity.to_owned(),
            passphrase: passphrase.as_str().to_owned(),
        },
    )
    .await
}

/// Write the unlocked identity's archive to `path`, owner-only.
async fn export_identity(
    client: &DaemonClient,
    path: &Path,
    passphrase_file: Option<&Path>,
) -> anyhow::Result<()> {
    let passphrase = helpers::read_passphrase("Passphrase", passphrase_file)?;
    // The same passphrase is the Protected review's proof.
    helpers::offer_passphrase(passphrase.clone());
    let value = helpers::send(
        client,
        IpcRequest::IdentityExport {
            passphrase: passphrase.as_str().to_owned(),
        },
    )
    .await?;
    let archive = value
        .get("archive")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("the daemon returned no archive"))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(archive)
        .map_err(|e| anyhow::anyhow!("the daemon's archive is not base64: {e}"))?;
    write_private(path, &bytes)?;
    format::print_text(&format!("Exported to {}", path.display()))
}

/// Install the identity archive at `path` (it stays locked).
async fn import_identity(
    client: &DaemonClient,
    path: &Path,
    passphrase_file: Option<&Path>,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let bytes =
        std::fs::read(path).map_err(|e| anyhow::anyhow!("cannot read {}: {e}", path.display()))?;
    let passphrase = helpers::read_passphrase("Passphrase of the archive", passphrase_file)?;
    let value = helpers::send(
        client,
        IpcRequest::IdentityImport {
            archive: base64::engine::general_purpose::STANDARD.encode(bytes),
            passphrase: passphrase.as_str().to_owned(),
        },
    )
    .await?;
    format::print_structured(&value, mode)
}

/// Write `bytes` to a new file at `path`, owner-only (it is an identity).
fn write_private(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|e| anyhow::anyhow!("cannot create {}: {e}", path.display()))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

pub async fn dispatch(
    cmd: &IdentityCmd,
    client: &DaemonClient,
    mode: OutputMode,
) -> anyhow::Result<()> {
    match cmd {
        IdentityCmd::Show { .. } => {
            let value = helpers::send(client, IpcRequest::IdentityShow).await?;
            format::print_structured(&value, mode)
        }
        IdentityCmd::List => {
            let value = helpers::send(client, IpcRequest::IdentityList).await?;
            format::print_structured(&value, mode)
        }
        IdentityCmd::Rotate { passphrase_file } => {
            let passphrase = helpers::read_passphrase("Passphrase", passphrase_file.as_deref())?;
            // The same passphrase is the Protected review's proof.
            helpers::offer_passphrase(passphrase.clone());
            let value = helpers::send(
                client,
                IpcRequest::IdentityRotate {
                    passphrase: passphrase.as_str().to_owned(),
                },
            )
            .await?;
            format::print_structured(&value, mode)
        }
        IdentityCmd::Delete {
            identity,
            passphrase_file,
        } => {
            // Protected: the host asks for the identity's passphrase.
            helpers::set_passphrase_file(passphrase_file.clone());
            let value = helpers::send(
                client,
                IpcRequest::IdentityDelete {
                    identity: identity.clone(),
                },
            )
            .await?;
            format::print_structured(&value, mode)
        }
        IdentityCmd::Export {
            path,
            passphrase_file,
        } => export_identity(client, path, passphrase_file.as_deref()).await,
        IdentityCmd::Import {
            path,
            passphrase_file,
        } => import_identity(client, path, passphrase_file.as_deref(), mode).await,
    }
}

pub async fn dispatch_export(
    cmd: &ExportCmd,
    client: &DaemonClient,
    _mode: OutputMode,
) -> anyhow::Result<()> {
    match cmd {
        ExportCmd::Identity {
            path,
            passphrase_file,
        } => export_identity(client, path, passphrase_file.as_deref()).await,
        ExportCmd::Friends { .. } => Err(unimplemented("export friends")),
        ExportCmd::Communities { .. } => Err(unimplemented("export communities")),
    }
}

pub async fn dispatch_import(
    cmd: &ImportCmd,
    client: &DaemonClient,
    mode: OutputMode,
) -> anyhow::Result<()> {
    match cmd {
        ImportCmd::Identity {
            path,
            passphrase_file,
        } => import_identity(client, path, passphrase_file.as_deref(), mode).await,
    }
}

/// A command that exists but does nothing yet: exit `EX_UNAVAILABLE` (69)
/// rather than pretend to succeed.
pub fn unimplemented(what: &str) -> anyhow::Error {
    anyhow::anyhow!(CliError::Unimplemented(what.to_owned()))
}
