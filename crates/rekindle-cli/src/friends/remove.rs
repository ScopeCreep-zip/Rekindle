//! `rekindle friend remove/block/unblock` — friend removal and blocking.

use anyhow::Context;

use rekindle_transport::operations::friend;
use rekindle_transport::Session;

use crate::helpers;
use crate::output::format;
use crate::output::OutputMode;
use crate::transport::TransportHandle;

/// Remove a friend with confirmation.
///
/// Sends an `Unfriend` DM notification (best-effort — the peer may be
/// offline), removes from the friend list DHT record, and invalidates
/// the cached route.
pub async fn cmd_remove(
    handle: &TransportHandle,
    session: &Session,
    friend_ref: &str,
    skip_confirm: bool,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let friend_ref = friend_ref.trim();
    if friend_ref.is_empty() {
        anyhow::bail!("friend identifier is required (display name or public key)");
    }

    if !skip_confirm {
        let confirmed = helpers::confirm(&format!(
            "Remove '{}'? They will no longer see your messages or presence.",
            helpers::abbreviate_key(friend_ref)
        ))?;
        if !confirmed {
            return format::print_text("Cancelled.");
        }
    }

    let signing_key = crate::identity::keystore::load_signing_key().await?;

    friend::remove_friend(handle.node(), session, friend_ref, &signing_key)
        .await
        .context("failed to remove friend")?;

    if mode.is_structured() {
        format::print_structured(
            &serde_json::json!({
                "status": "removed",
                "friend": friend_ref,
            }),
            mode,
        )
    } else {
        format::print_text(&format!(
            "Friend {} removed.",
            helpers::abbreviate_key(friend_ref)
        ))
    }
}

/// Block a peer — unfriend and suppress all future contact.
///
/// Currently delegates to remove_friend. Full blocking (suppressing
/// inbound DMs, hiding from searches, preventing re-add) requires
/// a block list in the session — the same pattern the Tauri app uses
/// with its `blocked_users` SQLite table.
pub async fn cmd_block(
    handle: &TransportHandle,
    session: &Session,
    friend_ref: &str,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let friend_ref = friend_ref.trim();
    if friend_ref.is_empty() {
        anyhow::bail!("peer identifier is required");
    }

    let signing_key = crate::identity::keystore::load_signing_key().await?;

    // Remove the friend (sends Unfriend notification)
    friend::remove_friend(handle.node(), session, friend_ref, &signing_key)
        .await
        .context("failed to block peer")?;

    // TODO: Add to blocked_peers list in Session when block list is implemented.
    // For now, removing the friend achieves the primary goal of cutting contact.

    if mode.is_structured() {
        format::print_structured(
            &serde_json::json!({
                "status": "blocked",
                "peer": friend_ref,
            }),
            mode,
        )
    } else {
        format::print_text(&format!(
            "Peer {} blocked.",
            helpers::abbreviate_key(friend_ref)
        ))
    }
}

/// Unblock a peer.
///
/// Currently a no-op because the block list isn't implemented yet.
/// The user can re-add the peer with `rekindle friend add`.
pub fn cmd_unblock(
    _handle: &TransportHandle,
    _session: &Session,
    friend_ref: &str,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let friend_ref = friend_ref.trim();
    if friend_ref.is_empty() {
        anyhow::bail!("peer identifier is required");
    }

    // TODO: Remove from blocked_peers list in Session when implemented.

    if mode.is_structured() {
        format::print_structured(
            &serde_json::json!({
                "status": "unblocked",
                "peer": friend_ref,
            }),
            mode,
        )
    } else {
        format::print_text(&format!(
            "Peer {} unblocked. Re-add with: rekindle friend add --target \"{}\"",
            helpers::abbreviate_key(friend_ref),
            friend_ref
        ))
    }
}
