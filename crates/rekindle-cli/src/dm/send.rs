//! `rekindle dm send` — send a direct message.

use anyhow::Context;

use rekindle_transport::operations::dm;
use rekindle_transport::Session;

use crate::helpers;
use crate::output::format;
use crate::output::OutputMode;
use crate::transport::TransportHandle;

/// Send a DM to a friend.
///
/// The friend identifier can be a display name or public key hex.
/// Route resolution uses the peer registry (cached routes) with
/// fallback to the peer's mailbox DHT record.
pub async fn cmd_send(
    handle: &TransportHandle,
    session: &Session,
    friend_ref: &str,
    message: &str,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let friend_ref = friend_ref.trim();
    if friend_ref.is_empty() {
        anyhow::bail!("friend identifier is required");
    }

    let message = message.trim();
    if message.is_empty() {
        anyhow::bail!("message body cannot be empty");
    }

    if message.len() > 2000 {
        anyhow::bail!(
            "message too long ({} chars, max 2000)",
            message.len()
        );
    }

    let signing_key = crate::identity::keystore::load_signing_key().await?;

    dm::send_dm(handle.node(), session, friend_ref, message, &signing_key)
        .await
        .context("failed to send DM")?;

    if mode.is_structured() {
        format::print_structured(
            &serde_json::json!({
                "status": "sent",
                "to": friend_ref,
                "length": message.len(),
            }),
            mode,
        )
    } else {
        format::print_text(&format!(
            "DM sent to {}.",
            helpers::abbreviate_key(friend_ref)
        ))
    }
}
