//! Sending one sealed, signed 1:1 envelope: the cached route, an inline
//! route refresh from the peer's profile, the Strand Relay fallback
//! (architecture §13.3), a status probe (§13.5), and finally the
//! pending-message queue. Every send goes through the node's ports (plan
//! E2.1); the bytes are [`super::wire`].

use rekindle_codec::message::envelope::{MessageEnvelope, MessagePayload};

use super::{route, seal, wire};
use crate::operations::friend::{state, FriendError, FriendHost};

/// Send `envelope`'s wire bytes over the peer's freshly fetched route.
async fn refresh_and_send(host: &dyn FriendHost, to: &str, bytes: &[u8]) -> bool {
    let Some(blob) = route::fetch_from_profile(host, to).await else {
        return false;
    };
    let Ok(node) = host.node() else {
        return false;
    };
    let Some(target) = route::import(&node, to, &blob) else {
        return false;
    };
    node.sender()
        .send_direct(&target, bytes.to_vec())
        .await
        .is_ok()
}

/// The Strand Relay fallback (architecture §13.3): when the cached route
/// and the inline refresh both fail, forward the envelope through a relay
/// from the recipient's published relay pool (profile subkey 8).
async fn relay_fallback(host: &dyn FriendHost, to: &str, envelope: &MessageEnvelope) -> bool {
    let Some(profile_key) = state::friend(host, to).and_then(|f| f.dht_record_key) else {
        return false;
    };
    let Ok(node) = host.node() else {
        return false;
    };
    let Ok(records) = node.require_records() else {
        return false;
    };
    let Ok(Some(pool_body)) = rekindle_protocol::dht::profile::read_profile_subkey(
        records.as_ref(),
        &profile_key,
        rekindle_protocol::dht::profile::SUBKEY_RELAY_POOL,
        true,
    )
    .await
    else {
        return false;
    };
    let Ok(envelope_bytes) = serde_json::to_vec(envelope) else {
        return false;
    };
    crate::operations::relay::send_via_relay(&node, to, &pool_body, &envelope_bytes)
        .await
        .is_ok()
}

/// Ask friends in the background whether any holds a fresh status and
/// route for `to` (architecture §13.5); a late answer refreshes the cache
/// for the next send.
fn probe_in_background(host: &dyn FriendHost, to: &str) {
    let owned = host.owned();
    let target = to.to_string();
    host.scope()
        .spawn_or_drop("relay status probe", async move {
            crate::operations::relay::probe_friends_for_status(owned.as_ref(), &target).await;
        });
}

/// Queue `envelope` for the pending-message retry.
///
/// # Errors
/// The queue could not be written.
pub(crate) async fn queue(
    host: &dyn FriendHost,
    to: &str,
    envelope: &MessageEnvelope,
) -> Result<(), FriendError> {
    let owner = state::owner(host)?;
    let body = serde_json::to_string(envelope)
        .map_err(|e| FriendError::Failed(format!("serialize envelope: {e}")))?;
    let recipient = to.to_string();
    let now = host.clock().now_ms_i64();
    state::db_call!(host, "queue pending message", move |conn| {
        rekindle_db::repo::pending_messages::insert(conn, &owner, &recipient, &body, now)
    })
    .await
}

/// Seal `payload` as its sealing requires, sign it to `to` and send it.
///
/// With no usable route the message is refreshed inline, relayed, and
/// otherwise queued for the retry. Ephemeral payloads (typing) are never
/// queued: a stale typing indicator minutes later is worse than none.
///
/// # Errors
/// The payload could not be sealed, or its envelope is over Veilid's
/// message cap (never queued: it fails on every path, plan C7.25).
pub async fn send_to_peer(
    host: &dyn FriendHost,
    to: &str,
    payload: &MessagePayload,
) -> Result<(), FriendError> {
    let is_ephemeral = matches!(payload, MessagePayload::TypingIndicator { .. });
    let envelope = seal::build_signed_envelope(host, to, payload).await?;
    let bytes = wire(&envelope)?;
    tracing::debug!(to = %to, wire_len = bytes.len(), "envelope fits the Veilid message cap");
    let node = host.node()?;

    let failure = match route::cached_target(&node, to) {
        Some(target) => match node.sender().send_direct(&target, bytes.clone()).await {
            Ok(()) => {
                tracing::info!(to = %to, "message sent via veilid");
                return Ok(());
            }
            Err(e) => {
                // The stale blob goes, so the retry fetches a fresh one.
                route::invalidate(&node, to);
                Some(e.to_string())
            }
        },
        None => None,
    };
    if is_ephemeral {
        tracing::debug!(to = %to, error = ?failure, "no route for peer — dropping ephemeral message");
        return Ok(());
    }
    if refresh_and_send(host, to, &bytes).await {
        tracing::info!(to = %to, "message sent via veilid (after inline route refresh)");
        return Ok(());
    }
    if relay_fallback(host, to, &envelope).await {
        tracing::info!(to = %to, "message sent via strand relay");
        return Ok(());
    }
    if failure.is_none() {
        probe_in_background(host, to);
    }
    tracing::debug!(to = %to, error = ?failure, "no usable route for peer — queuing message for retry");
    queue(host, to, &envelope).await
}

/// Seal and sign `payload` and queue it for the retry without trying to
/// send it now: a notice whose delivery the queue must track (`Unfriended`
/// until its ack, an ack the peer is waiting on).
///
/// # Errors
/// The payload could not be sealed, or the queue could not be written.
pub async fn build_and_queue(
    host: &dyn FriendHost,
    to: &str,
    payload: &MessagePayload,
) -> Result<(), FriendError> {
    let envelope = seal::build_signed_envelope(host, to, payload).await?;
    queue(host, to, &envelope).await
}

/// `app_call` `payload` to `to` and await the reply, a raw `MessagePayload`
/// JSON (the receiver shapes it directly).
///
/// # Errors
/// No cached route, the call failed, or the reply does not decode.
pub async fn send_to_peer_call(
    host: &dyn FriendHost,
    to: &str,
    payload: &MessagePayload,
) -> Result<MessagePayload, FriendError> {
    let envelope = seal::build_signed_envelope(host, to, payload).await?;
    let bytes = wire(&envelope)?;
    let node = host.node()?;
    let target = route::cached_target(&node, to)
        .ok_or_else(|| FriendError::NotReady(format!("no cached route for peer {to}")))?;
    let reply = node
        .caller()
        .call_direct(&target, bytes)
        .await
        .map_err(|e| FriendError::Failed(format!("app_call: {e}")))?;
    serde_json::from_slice::<MessagePayload>(&reply)
        .map_err(|e| FriendError::Failed(format!("decode reply payload: {e}")))
}

/// What one retry of a queued envelope came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resend {
    /// It went out.
    Delivered,
    /// No route, or the send failed: try again next tick.
    Failed,
    /// It cannot be re-signed (its recipient is not a key, or the identity
    /// is gone): drop it.
    Unsignable,
}

/// Send a queued envelope again over the peer's cached route, or their
/// mailbox's route when none is cached, re-signed with a fresh timestamp
/// and the same nonce: the receiver's freshness window would reject the
/// original signature, and a copy that already arrived is dropped as a
/// duplicate (the pending-message retry).
pub async fn resend(host: &dyn FriendHost, to: &str, envelope: &MessageEnvelope) -> Resend {
    let Ok(node) = host.node() else {
        return Resend::Failed;
    };
    let target = if let Some(target) = route::cached_target(&node, to) {
        target
    } else {
        let Some(blob) = route::fetch_from_mailbox(host, to).await else {
            return Resend::Failed;
        };
        let Some(target) = route::import(&node, to, &blob) else {
            return Resend::Failed;
        };
        tracing::debug!(to = %to, "discovered route via mailbox fallback");
        target
    };
    let bytes = match seal::resign_for_retry(host, to, envelope).and_then(|e| wire(&e)) {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::warn!(to = %to, %error, "pending DM cannot be re-signed — dropping");
            return Resend::Unsignable;
        }
    };
    match node.sender().send_direct(&target, bytes).await {
        Ok(()) => {
            tracing::debug!(to = %to, "pending DM delivered successfully");
            Resend::Delivered
        }
        Err(error) => {
            tracing::debug!(to = %to, %error, "pending DM retry failed");
            Resend::Failed
        }
    }
}
