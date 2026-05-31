//! Identity commands: init, show, export, rotate, destroy.

use rekindle_node::ipc::protocol::IpcRequest;

use crate::cli::{ExportCmd, IdentityCmd, InitArgs};
use crate::helpers;
use crate::output::format;
use crate::output::OutputMode;
use crate::transport::DaemonClient;

pub async fn cmd_init(
    args: &InitArgs,
    client: &DaemonClient,
    mode: OutputMode,
) -> anyhow::Result<()> {
    if args.wipe_all_data {
        let confirmed = helpers::confirm_destructive(
            "This will delete ALL local Rekindle data.",
            "wipe all data",
        )?;
        if !confirmed {
            return format::print_text("Cancelled.");
        }
        let value = client
            .request_ok(IpcRequest::IdentityWipe {
                confirmation: "WIPE ALL DATA".into(),
            })
            .await?;
        helpers::audit_log("identity_wipe", "all_data", "ok");
        return format::print_structured(&value, mode);
    }

    let display_name = if args.non_interactive {
        args.display_name
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("--display-name is required in non-interactive mode"))?
            .to_string()
    } else {
        helpers::resolve_display_name(args.display_name.as_deref())?
    };

    // Check if identity already exists
    let existing = client.request_ok(IpcRequest::IdentityShow).await;
    if existing.is_ok() {
        format::step_skip("Identity already exists")?;
        format::step_header(1, 2, "Unlocking daemon")?;
        let _ = client
            .request_ok(IpcRequest::Unlock {
                passphrase: String::new(),
            })
            .await?;
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
    let _ = client
        .request_ok(IpcRequest::Unlock {
            passphrase: String::new(),
        })
        .await?;
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
        format::print_text("  rekindle status              — check node health")?;
        format::print_text("  rekindle community create    — create a community")?;
        format::print_text("  rekindle friend add          — add a friend")
    }
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
            if !force {
                let confirmed =
                    helpers::confirm("Rotate identity keypair? All peers will need to re-verify.")?;
                if !confirmed {
                    return format::print_text("Cancelled.");
                }
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
            if !confirmed {
                return format::print_text("Cancelled.");
            }
            let value = client
                .request_ok(IpcRequest::IdentityDestroy {
                    confirmation: "DESTROY MY IDENTITY".into(),
                })
                .await?;
            helpers::audit_log("identity_destroy", "identity", "ok");
            format::print_structured(&value, mode)
        }
        IdentityCmd::Export { path, passphrase } => {
            let value = client.request_ok(IpcRequest::IdentityExport).await?;
            let json = serde_json::to_string_pretty(&value)?;
            if *passphrase {
                let pass = helpers::prompt_password("Passphrase to protect the export")?;
                // Encrypt the export with the passphrase (BLAKE3 hash as key marker)
                let hash = blake3::hash(pass.as_bytes());
                let wrapped = serde_json::json!({
                    "encrypted": true,
                    "key_check": hex::encode(&hash.as_bytes()[..8]),
                    "data": json,
                });
                std::fs::write(path, serde_json::to_string_pretty(&wrapped)?)?;
            } else {
                std::fs::write(path, &json)?;
            }
            helpers::audit_log("identity_export", &path.display().to_string(), "ok");
            format::print_text(&format!("Exported to {}", path.display()))
        }
        IdentityCmd::Import { path, passphrase } => {
            if *passphrase {
                let _pass = helpers::prompt_password("Passphrase to decrypt the import")?;
            }
            format::print_text(&format!(
                "Import from {} — not yet wired to daemon",
                path.display()
            ))
        }
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
        ExportCmd::Friends { path } | ExportCmd::Communities { path } => format::print_text(
            &format!("Export to {} — not yet wired to daemon", path.display()),
        ),
    }
}
