//! Phase 23.D — outgoing-message public API lifted from
//! `message_service/mod.rs`. All `send_*` entry points that callers
//! outside `message_service` use live here; envelope construction +
//! per-peer dispatch + pending-message queue. The lower-level
//! `send_envelope_to_peer` orchestrator (and its DHT-route +
//! relay-fallback helpers) stays in `mod.rs` for now until the
//! transport split lands.

use std::sync::Arc;

use rand::RngCore as _;
use rekindle_protocol::messaging::envelope::MessagePayload;
use rekindle_protocol::messaging::sender::build_envelope_from_secret;

use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::state::AppState;
use crate::state_helpers;

/// Send `payload` to `to` via Veilid `app_call`, await the reply, and
/// return the deserialized reply envelope. Used for the DM accept/decline
/// handshake (architecture §27.1) and other cases where the caller needs
/// a guaranteed reply rather than queue-on-failure.
pub async fn send_to_peer_call(
    state: &Arc<AppState>,
    to: &str,
    payload: &MessagePayload,
) -> Result<MessagePayload, String> {
    let payload_bytes =
        serde_json::to_vec(payload).map_err(|e| format!("serialize payload: {e}"))?;
    let secret_key = {
        let sk = state.identity_secret.lock();
        *sk.as_ref().ok_or("signing key not initialized")?
    };
    let timestamp = rekindle_utils::timestamp_ms();
    let nonce = {
        let mut buf = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut buf);
        buf.to_vec()
    };
    let envelope = build_envelope_from_secret(&secret_key, timestamp, nonce, payload_bytes);

    let (route_id, routing_context) = state_helpers::try_import_peer_route(state, to)
        .ok_or_else(|| format!("no cached route for peer {to}"))?;

    let reply_bytes =
        rekindle_protocol::messaging::sender::send_call(&routing_context, route_id, &envelope)
            .await
            .map_err(|e| format!("app_call: {e}"))?;

    // Replies are raw `MessagePayload` JSON (the receiver shapes their
    // reply directly, not as a full signed envelope).
    serde_json::from_slice::<MessagePayload>(&reply_bytes)
        .map_err(|e| format!("decode reply payload: {e}"))
}

/// Send a direct message to a peer via the Veilid network.
pub async fn send_message(
    state: &Arc<AppState>,
    pool: &DbPool,
    to: &str,
    body: &str,
) -> Result<(), String> {
    let payload = MessagePayload::DirectMessage {
        body: body.to_string(),
        reply_to: None,
    };
    // Encrypt DMs when a Signal session exists
    super::transport::send_envelope_to_peer(state, pool, to, &payload, true).await
}

/// Send a friend request to a peer via Veilid.
///
/// Includes our `PreKeyBundle` so the receiver can establish a Signal session.
/// Sent unencrypted (no session with the peer yet).
pub async fn send_friend_request(
    state: &Arc<AppState>,
    pool: &DbPool,
    to: &str,
    message: &str,
    invite_id: Option<&str>,
) -> Result<(), String> {
    let display_name = state_helpers::current_identity(state)
        .map_err(|_| "identity not set".to_string())?
        .display_name;

    let prekey_bundle = {
        let signal = state.signal_manager.read();
        if let Some(handle) = signal.as_ref() {
            match handle.manager.generate_prekey_bundle(1, Some(1), Some(1)) {
                Ok(bundle) => serde_json::to_vec(&bundle).unwrap_or_default(),
                Err(e) => {
                    tracing::warn!(error = %e, "failed to generate PreKeyBundle for friend request");
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        }
    };

    // Gather our profile and mailbox DHT keys + route blob for the invite payload.
    //
    // B4/P3.2 — if our route_blob hasn't been allocated yet (e.g. user hits
    // "Add friend" within seconds of login, before Veilid finishes private
    // route setup), wait up to 5 seconds for it to land. Sending the request
    // with an empty blob meant the receiver had no inbound path to reply on
    // and the friend handshake silently stalled.
    let (profile_dht_key, route_blob, mailbox_dht_key) = {
        let read_nh = || -> Result<(String, Vec<u8>, String), String> {
            let node = state.node.read();
            let nh = node.as_ref().ok_or("node not initialized")?;
            Ok((
                nh.profile_dht_key.clone().unwrap_or_default(),
                nh.route_blob.clone().unwrap_or_default(),
                nh.mailbox_dht_key.clone().unwrap_or_default(),
            ))
        };
        let mut tuple = read_nh()?;
        if tuple.1.is_empty() {
            const POLL_INTERVAL_MS: u64 = 100;
            const MAX_WAIT_MS: u64 = 5_000;
            let mut waited_ms = 0_u64;
            while tuple.1.is_empty() && waited_ms < MAX_WAIT_MS {
                tokio::time::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS)).await;
                waited_ms += POLL_INTERVAL_MS;
                tuple = read_nh()?;
            }
            if tuple.1.is_empty() {
                return Err(
                    "Veilid private route not allocated yet — try again in a moment".to_string(),
                );
            }
            tracing::info!(
                waited_ms,
                "send_friend_request: route blob became available after wait"
            );
        }
        tuple
    };

    tracing::info!(
        to = %to,
        route_blob_len = route_blob.len(),
        route_count = route_blob.first().copied().unwrap_or(0),
        "send_friend_request: our route blob info"
    );

    let payload = MessagePayload::FriendRequest {
        display_name,
        message: message.to_string(),
        prekey_bundle,
        profile_dht_key,
        route_blob,
        mailbox_dht_key,
        invite_id: invite_id.map(str::to_string),
    };
    // Friend requests are NOT encrypted (no session yet)
    super::transport::send_envelope_to_peer(state, pool, to, &payload, false).await
}

/// Send a friend acceptance to a peer via Veilid.
///
/// Includes our `PreKeyBundle` and (if available) the `SessionInitInfo` from
/// `establish_session()` so the requester can call `respond_to_session()`.
pub async fn send_friend_accept(
    state: &Arc<AppState>,
    pool: &DbPool,
    to: &str,
    session_init: Option<rekindle_crypto::signal::SessionInitInfo>,
) -> Result<(), String> {
    let prekey_bundle = {
        let signal = state.signal_manager.read();
        if let Some(handle) = signal.as_ref() {
            match handle.manager.generate_prekey_bundle(1, Some(1), Some(1)) {
                Ok(bundle) => serde_json::to_vec(&bundle).unwrap_or_default(),
                Err(e) => {
                    tracing::warn!(error = %e, "failed to generate PreKeyBundle for friend accept");
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        }
    };

    // Gather our profile and mailbox DHT keys + route blob
    let (profile_dht_key, route_blob, mailbox_dht_key) = {
        let node = state.node.read();
        let nh = node.as_ref().ok_or("node not initialized")?;
        (
            nh.profile_dht_key.clone().unwrap_or_default(),
            nh.route_blob.clone().unwrap_or_default(),
            nh.mailbox_dht_key.clone().unwrap_or_default(),
        )
    };

    if route_blob.is_empty() {
        tracing::warn!(
            "sending friend accept with empty route blob — peer will fetch from DHT profile"
        );
    }

    let payload = MessagePayload::FriendAccept {
        prekey_bundle,
        profile_dht_key,
        route_blob,
        mailbox_dht_key,
        ephemeral_key: session_init
            .as_ref()
            .map(|s| s.ephemeral_public_key.clone())
            .unwrap_or_default(),
        signed_prekey_id: session_init.as_ref().map_or(1, |s| s.signed_prekey_id),
        one_time_prekey_id: session_init.as_ref().and_then(|s| s.one_time_prekey_id),
        ml_kem_ciphertext: session_init
            .as_ref()
            .map(|s| s.ml_kem_ciphertext.clone())
            .unwrap_or_default(),
        used_ot_pqpk_id: session_init.as_ref().and_then(|s| s.used_ot_pqpk_id),
    };
    // Friend accepts are NOT encrypted (the requester may not have our session yet)
    super::transport::send_envelope_to_peer(state, pool, to, &payload, false).await
}

/// Send a friend rejection to a peer via Veilid.
pub async fn send_friend_reject(
    state: &Arc<AppState>,
    pool: &DbPool,
    to: &str,
) -> Result<(), String> {
    let payload = MessagePayload::FriendReject;
    // Rejections are NOT encrypted
    super::transport::send_envelope_to_peer(state, pool, to, &payload, false).await
}

/// Send a typing indicator to a peer.
pub async fn send_typing(
    state: &Arc<AppState>,
    pool: &DbPool,
    to: &str,
    typing: bool,
) -> Result<(), String> {
    let payload = MessagePayload::TypingIndicator { typing };
    // Typing indicators use encryption if session exists
    super::transport::send_envelope_to_peer(state, pool, to, &payload, true).await
}

/// Send a raw (unencrypted) payload to a peer.
///
/// Used for protocol-level messages like `ProfileKeyRotated` that don't need
/// E2E encryption (the content isn't secret — it's a public DHT key).
pub async fn send_to_peer_raw(
    state: &Arc<AppState>,
    pool: &DbPool,
    to: &str,
    payload: &MessagePayload,
) -> Result<(), String> {
    super::transport::send_envelope_to_peer(state, pool, to, payload, false).await
}

/// W11.4 — encrypted-only public wrapper for DM payloads that MUST go
/// through the Signal Double Ratchet (DM video fragments,
/// future encrypted control messages). Mirrors `send_to_peer_raw`
/// but with `encrypt = true`. Fails closed (no plaintext fallback)
/// if no Signal session exists — caller surfaces the error so the
/// user is offered the explicit re-handshake path.
pub async fn send_to_peer_encrypted(
    state: &Arc<AppState>,
    pool: &DbPool,
    to: &str,
    payload: &MessagePayload,
) -> Result<(), String> {
    super::transport::send_envelope_to_peer(state, pool, to, payload, true).await
}

/// Build a signed `MessageEnvelope` for the given payload and queue it in
/// `pending_messages` for retry by `sync_service`.
///
/// Used by `friends.rs` to always-queue an `Unfriended` message regardless of
/// whether the initial `send_to_peer_raw` succeeded (Veilid `app_message` has
/// no delivery guarantee). The queued entry is cleared when the peer sends an
/// `UnfriendedAck`, or dropped after max retries (20 x 30s).
pub(crate) async fn build_and_queue_envelope(
    state: &Arc<AppState>,
    pool: &DbPool,
    to: &str,
    payload: &MessagePayload,
) -> Result<(), String> {
    let payload_bytes =
        serde_json::to_vec(payload).map_err(|e| format!("serialize payload: {e}"))?;

    let secret_key = {
        let sk = state.identity_secret.lock();
        *sk.as_ref().ok_or("signing key not initialized")?
    };

    let timestamp = rekindle_utils::timestamp_ms();

    let nonce = {
        let mut buf = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut buf);
        buf.to_vec()
    };

    let envelope = build_envelope_from_secret(&secret_key, timestamp, nonce, payload_bytes);
    let envelope_json =
        serde_json::to_string(&envelope).map_err(|e| format!("serialize envelope: {e}"))?;
    queue_pending_message(state, pool, to, &envelope_json).await
}

/// Insert a message into the `pending_messages` table for later retry.
pub(super) async fn queue_pending_message(
    state: &Arc<AppState>,
    pool: &DbPool,
    recipient_key: &str,
    body: &str,
) -> Result<(), String> {
    let owner_key = state_helpers::owner_key_or_default(state);
    let recipient = recipient_key.to_string();
    let body = body.to_string();
    let now = crate::db::timestamp_now();
    db_call(pool, move |conn| {
        conn.execute(
            "INSERT INTO pending_messages (owner_key, recipient_key, body, created_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![owner_key, recipient, body, now],
        )?;
        Ok(())
    })
    .await
}
