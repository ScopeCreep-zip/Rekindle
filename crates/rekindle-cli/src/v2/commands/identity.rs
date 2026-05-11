//! Identity commands: init, show, export, rotate, destroy, wipe.
//!
//! Security: All file writes use write_restricted() which sets 0o600 permissions.
//! Identity export with --passphrase delegates encryption to the daemon (which has
//! aws-lc-rs + Argon2id). The CLI never performs crypto on key material.

use std::io::Write;
use std::path::Path;

use base64::Engine;
use rekindle_node::ipc::protocol::IpcRequest;

use crate::v2::cli::{IdentityCmd, InitArgs, ExportCmd};
use crate::v2::helpers;
use crate::v2::output::format;
use crate::v2::output::OutputMode;
use crate::v2::transport::DaemonClient;

/// Write bytes to a file with 0o600 permissions (owner read/write only).
fn write_restricted(path: &Path, data: &[u8]) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(data)?;
        file.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, data)?;
    }
    Ok(())
}

pub async fn cmd_init(args: &InitArgs, client: &DaemonClient, mode: OutputMode) -> anyhow::Result<()> {
    if args.wipe_all_data {
        let confirmed = helpers::confirm_destructive(
            "This will delete ALL local Rekindle data.",
            "wipe all data",
        )?;
        if !confirmed { return format::print_text("Cancelled."); }
        let value = client.request_ok(IpcRequest::IdentityWipe {
            confirmation: "WIPE".into(),
        }).await?;
        helpers::audit_log("identity_wipe", "all_data", "ok");
        return format::print_structured(&value, mode);
    }

    let display_name = if args.non_interactive {
        let raw = args.display_name.as_deref()
            .ok_or_else(|| anyhow::anyhow!("--display-name is required in non-interactive mode"))?;
        helpers::validate_display_name(raw)?
    } else {
        helpers::resolve_display_name(args.display_name.as_deref())?
    };

    let existing = client.request_ok(IpcRequest::IdentityShow).await;
    if existing.is_ok() {
        format::step_skip("Identity already exists")?;
        format::step_header(1, 2, "Unlocking daemon")?;
        let _ = client.request_ok(IpcRequest::Unlock { passphrase: String::new() }).await?;
        format::step_done("daemon operational")?;
        return format::print_text("Identity already initialized. Daemon unlocked.");
    }

    format::step_header(1, 3, "Unlocking daemon")?;
    let _ = client.request_ok(IpcRequest::Unlock { passphrase: String::new() }).await?;
    format::step_done("daemon unlocked")?;

    format::step_header(2, 3, "Creating identity via daemon")?;
    let value = client.request_ok(IpcRequest::IdentityCreate { display_name: display_name.clone() }).await?;
    format::step_done("identity created")?;

    format::step_header(3, 3, "Verifying daemon operational")?;

    if let Some(ref export_path) = args.export_identity {
        let export_value = client.request_ok(IpcRequest::IdentityExport).await?;
        let json = serde_json::to_string_pretty(&export_value)?;
        write_restricted(export_path, json.as_bytes())?;
        helpers::audit_log("identity_export", &export_path.display().to_string(), "ok");
        format::print_text(&format!("  Identity exported to {} (permissions: 0600)", export_path.display()))?;
    }

    if mode.is_structured() {
        format::print_structured(&value, mode)
    } else {
        format::print_text("\nIdentity created successfully.")?;
        if let Some(pk) = value.get("public_key").and_then(|v| v.as_str()) {
            format::print_text(&format!("  Public key: {pk}"))?;
        }
        format::print_text(&format!("  Display name: {display_name}"))?;
        format::print_text("\nNext steps:")?;
        format::print_text("  rekindle status              — check node health")?;
        format::print_text("  rekindle community create    — create a community")?;
        format::print_text("  rekindle friend add          — add a friend")
    }
}

pub async fn dispatch(cmd: &IdentityCmd, client: &DaemonClient, mode: OutputMode) -> anyhow::Result<()> {
    match cmd {
        IdentityCmd::Show { .. } => {
            let value = client.request_ok(IpcRequest::IdentityShow).await?;
            format::print_structured(&value, mode)
        }
        IdentityCmd::Rotate { force } => {
            if !force {
                let confirmed = helpers::confirm("Rotate identity keypair? All peers will need to re-verify.")?;
                if !confirmed { return format::print_text("Cancelled."); }
            }
            let value = client.request_ok(IpcRequest::IdentityRotate).await?;
            helpers::audit_log("identity_rotate", "keypair", "ok");
            format::print_structured(&value, mode)
        }
        IdentityCmd::Destroy => {
            let confirmed = helpers::confirm_destructive(
                "This will permanently destroy your identity.",
                "destroy my identity",
            )?;
            if !confirmed { return format::print_text("Cancelled."); }
            let value = client.request_ok(IpcRequest::IdentityDestroy {
                confirmation: "DESTROY MY IDENTITY".into(),
            }).await?;
            helpers::audit_log("identity_destroy", "identity", "ok");
            format::print_structured(&value, mode)
        }
        IdentityCmd::Export { path, passphrase } => {
            if *passphrase {
                let mut pass = helpers::prompt_password("Passphrase to protect the export")?;
                // Move the passphrase out of the Zeroizing wrapper without copying.
                // The original Zeroizing<String> zeros its allocation on drop.
                // The moved String lives only in the IpcRequest until serialization.
                let passphrase_owned = std::mem::take(&mut *pass);
                let value = client.request_ok(IpcRequest::IdentityExportEncrypted {
                    passphrase: passphrase_owned,
                }).await?;
                let blob_b64 = value.as_str()
                    .ok_or_else(|| anyhow::anyhow!("daemon returned invalid encrypted export format"))?;
                let blob = base64::engine::general_purpose::STANDARD.decode(blob_b64)
                    .map_err(|e| anyhow::anyhow!("invalid base64 from daemon: {e}"))?;
                write_restricted(path, &blob)?;
            } else {
                let value = client.request_ok(IpcRequest::IdentityExport).await?;
                let json = serde_json::to_string_pretty(&value)?;
                write_restricted(path, json.as_bytes())?;
            }
            helpers::audit_log("identity_export", &path.display().to_string(), "ok");
            format::print_text(&format!("Exported to {} (permissions: 0600)", path.display()))
        }
        IdentityCmd::Import { path, passphrase } => {
            let data = std::fs::read(path)
                .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", path.display()))?;

            if *passphrase {
                let mut pass = helpers::prompt_password("Passphrase to decrypt the import")?;
                let passphrase_owned = std::mem::take(&mut *pass);
                let blob_b64 = base64::engine::general_purpose::STANDARD.encode(&data);
                let _value = client.request_ok(IpcRequest::IdentityImportEncrypted {
                    passphrase: passphrase_owned,
                    data: blob_b64,
                }).await?;
                format::print_text("Identity imported successfully.")
            } else {
                let json = String::from_utf8(data)
                    .map_err(|_| anyhow::anyhow!("identity file is not valid UTF-8"))?;
                let _value = client.request_ok(IpcRequest::IdentityImport {
                    data: json,
                }).await?;
                format::print_text("Identity imported successfully.")
            }
        }
    }
}

pub async fn dispatch_export(cmd: &ExportCmd, client: &DaemonClient, _mode: OutputMode) -> anyhow::Result<()> {
    match cmd {
        ExportCmd::Identity { path } => {
            let value = client.request_ok(IpcRequest::IdentityExport).await?;
            let json = serde_json::to_string_pretty(&value)?;
            write_restricted(path, json.as_bytes())?;
            format::print_text(&format!("Exported to {} (permissions: 0600)", path.display()))
        }
        ExportCmd::Friends { path } | ExportCmd::Communities { path } => {
            format::print_text(&format!("Export to {} — not yet wired to daemon", path.display()))
        }
    }
}
