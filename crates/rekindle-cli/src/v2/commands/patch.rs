//! Patch commands — generate from local git, apply from file, send as message.

use std::path::Path;

use crate::v2::output::format;
use crate::v2::output::OutputMode;
use crate::v2::patch::{apply, generate};
use crate::v2::transport::DaemonClient;

/// `rekindle patch [files...] [--staged] [--message] [--channel-community -C channel] [--dm-peer]`
pub async fn cmd_patch(
    files: &[String],
    staged: bool,
    channel_community: Option<&str>,
    channel_name: Option<&str>,
    dm_peer: Option<&str>,
    message: Option<&str>,
    client: Option<&DaemonClient>,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;

    // Generate the patch
    let file_refs: Vec<&str> = files.iter().map(String::as_str).collect();
    let mut patch = if staged {
        generate::generate_staged_patch()?
    } else if file_refs.is_empty() {
        generate::generate_patch(&cwd, &[], false)?
    } else {
        generate::generate_patch(&cwd, &file_refs, false)?
    };

    if patch.diff.trim().is_empty() {
        return format::print_text("No changes to create a patch from.");
    }

    if let Some(msg) = message {
        patch.description = Some(msg.to_string());
    }

    // If sending to a channel or DM, construct the message body with patch fence
    let should_send = channel_community.is_some() || dm_peer.is_some();

    if should_send {
        let Some(client) = client else {
            anyhow::bail!("--channel or --dm-peer requires a daemon connection");
        };

        let desc = patch.description.as_deref().unwrap_or("");
        let body = if desc.is_empty() {
            format!("```patch\n{}\n```", patch.diff)
        } else {
            format!("{desc}\n\n```patch\n{}\n```", patch.diff)
        };

        if let (Some(community), Some(channel)) = (channel_community, channel_name) {
            let value = client.request_ok(rekindle_node::ipc::protocol::IpcRequest::ChannelSend {
                community: community.to_string(),
                channel: channel.to_string(),
                body,
                reply_to: None,
                client_msg_id: None,
            }).await?;
            if mode.is_structured() {
                return format::print_structured(&value, mode);
            }
            format::print_text(&format!(
                "Patch sent to #{channel} ({}) — {}",
                crate::v2::helpers::abbreviate_key(community),
                patch.summary(),
            ))
        } else if let Some(peer) = dm_peer {
            let value = client.request_ok(rekindle_node::ipc::protocol::IpcRequest::DmSend {
                peer_key: peer.to_string(),
                body,
            }).await?;
            if mode.is_structured() {
                return format::print_structured(&value, mode);
            }
            format::print_text(&format!(
                "Patch sent to {} — {}",
                crate::v2::helpers::abbreviate_key(peer),
                patch.summary(),
            ))
        } else {
            anyhow::bail!("--channel-community requires --channel-name, or use --dm-peer")
        }
    } else {
        // Print patch to stdout
        if mode.is_structured() {
            return format::print_structured(&patch, mode);
        }

        format::print_text(&format!("# {}", patch.summary()))?;
        if let Some(ref desc) = patch.description {
            format::print_text(&format!("# {desc}"))?;
        }
        if let Some(ref base) = patch.base_ref {
            format::print_text(&format!("# base: {base}"))?;
        }
        format::print_text("")?;

        // Write raw diff to stdout
        let mut stdout = std::io::stdout().lock();
        use std::io::Write;
        write!(stdout, "{}", patch.diff)?;
        Ok(())
    }
}

/// `rekindle patch-apply <path> [--check]`
pub fn cmd_patch_apply(
    path: &Path,
    check_only: bool,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let diff_text = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("cannot read patch file {}: {e}", path.display()))?;

    let patch = crate::v2::patch::render::parse_diff_to_patch(&diff_text);

    if patch.files.is_empty() {
        anyhow::bail!("no valid diff content found in {}", path.display());
    }

    let cwd = std::env::current_dir()?;

    if check_only {
        match apply::check_patch(&cwd, &patch) {
            None => {
                if mode.is_structured() {
                    return format::print_structured(&serde_json::json!({"clean": true}), mode);
                }
                format::print_text(&format!("Patch applies cleanly — {}", patch.summary()))
            }
            Some(error) => {
                if mode.is_structured() {
                    return format::print_structured(&serde_json::json!({"clean": false, "error": error}), mode);
                }
                anyhow::bail!("Patch would not apply cleanly: {error}")
            }
        }
    } else {
        let result = apply::apply_patch(&cwd, &patch);

        if let Some(ref error) = result.error {
            if mode.is_structured() {
                return format::print_structured(&serde_json::json!({
                    "success": false,
                    "error": error,
                    "clean_apply": result.clean_apply,
                    "local_head": result.local_head,
                }), mode);
            }
            anyhow::bail!("Patch application failed: {error}");
        }

        if mode.is_structured() {
            return format::print_structured(&serde_json::json!({
                "success": true,
                "files": result.applied_files,
                "clean_apply": result.clean_apply,
                "local_head": result.local_head,
            }), mode);
        }

        if !result.clean_apply {
            format::print_text("Warning: patch was generated against a different commit.")?;
            if let Some(ref head) = result.local_head {
                format::print_text(&format!("  local HEAD: {head}"))?;
            }
        }

        for file in &result.applied_files {
            format::print_text(&format!("  patched: {file}"))?;
        }
        format::print_text(&format!("\n{}", patch.summary()))
    }
}
