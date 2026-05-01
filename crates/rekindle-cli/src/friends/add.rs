//! `rekindle friend add` — send a friend request.

use anyhow::Context;

use rekindle_transport::operations::friend;
use rekindle_transport::Session;

use crate::helpers;
use crate::output::format;
use crate::output::OutputMode;
use crate::transport::TransportHandle;

/// Send a friend request to a target peer.
///
/// The target can be a public key hex string, a mailbox DHT key, or
/// (in the future) a display name search. For now, the target is treated
/// as a mailbox DHT key — the key published in the peer's profile that
/// allows receiving friend requests.
pub async fn cmd_add(
    handle: &TransportHandle,
    session: &Session,
    target: &str,
    message: Option<&str>,
    mode: OutputMode,
) -> anyhow::Result<()> {
    if target.trim().is_empty() {
        anyhow::bail!("target is required (public key or mailbox key)");
    }

    let signing_key = crate::identity::keystore::load_signing_key().await?;
    let request_message = message.unwrap_or("Hello! Let's connect on Rekindle.");

    friend::send_friend_request(
        handle.node(),
        session,
        target.trim(),
        request_message,
        &signing_key,
    )
    .await
    .context("failed to send friend request")?;

    if mode.is_structured() {
        format::print_structured(
            &serde_json::json!({
                "status": "sent",
                "target": target,
                "message": request_message,
            }),
            mode,
        )
    } else {
        format::print_text(&format!(
            "Friend request sent to {}.",
            helpers::abbreviate_key(target)
        ))?;
        format::print_text("  They'll appear in your friend list once they accept.")
    }
}
