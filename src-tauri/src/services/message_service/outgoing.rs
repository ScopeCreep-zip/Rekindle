//! Phase 23.D — outgoing-message public API lifted from
//! `message_service/mod.rs`. All `send_*` entry points that callers
//! outside `message_service` use live here; envelope construction +
//! per-peer dispatch + pending-message queue. The lower-level
//! `send_envelope_to_peer` orchestrator (and its DHT-route +
//! relay-fallback helpers) stays in `mod.rs` for now until the
//! transport split lands.

use std::sync::Arc;

use rekindle_codec::message::envelope::MessagePayload;

use crate::db_helpers::db_call;
use crate::state::AppState;
use crate::state_helpers;
use rekindle_db::Db;

/// Send `payload` to `to` via Veilid `app_call`, await the reply, and
/// return the deserialized reply envelope. Used for the DM accept/decline
/// handshake (architecture §27.1) and other cases where the caller needs
/// a guaranteed reply rather than queue-on-failure.
pub async fn send_to_peer_call(
    state: &Arc<AppState>,
    to: &str,
    payload: &MessagePayload,
) -> Result<MessagePayload, String> {
    let envelope = super::seal::build_signed_envelope(state, to, payload).await?;

    let (route_id, routing_context) = state_helpers::try_import_peer_route(state, to)
        .ok_or_else(|| format!("no cached route for peer {to}"))?;

    let reply = rekindle_protocol::messaging::sender::send_call(
        &routing_context,
        route_id.clone(),
        &envelope,
    )
    .await;
    let reply_bytes = state_helpers::note_send_result(state, &route_id, reply)
        .map_err(|e| format!("app_call: {e}"))?;

    // Replies are raw `MessagePayload` JSON (the receiver shapes their
    // reply directly, not as a full signed envelope).
    serde_json::from_slice::<MessagePayload>(&reply_bytes)
        .map_err(|e| format!("decode reply payload: {e}"))
}

/// Send a direct message to a peer via the Veilid network.
pub async fn send_message(
    state: &Arc<AppState>,
    pool: &Db,
    to: &str,
    body: &str,
) -> Result<(), String> {
    let payload = MessagePayload::DirectMessage {
        body: body.to_string(),
        reply_to: None,
    };
    super::transport::send_envelope_to_peer(state, pool, to, &payload).await
}

/// A serialized bundle carrying fresh one-time keys, for exactly one peer.
fn handout_bundle_bytes(state: &Arc<AppState>) -> Result<Vec<u8>, String> {
    let signal = state.signal_manager.read();
    let handle = signal.as_ref().ok_or("signal manager not initialized")?;
    let bundle = handle
        .manager
        .handout_bundle()
        .map_err(|e| format!("prekey bundle: {e}"))?;
    serde_json::to_vec(&bundle).map_err(|e| format!("serialize prekey bundle: {e}"))
}

/// Send a friend request to a peer via Veilid.
///
/// Includes our `PreKeyBundle` so the receiver can establish a Signal session.
/// Signed but not encrypted (`Sealing::Plain`): there is no session yet.
pub async fn send_friend_request(
    state: &Arc<AppState>,
    pool: &Db,
    to: &str,
    message: &str,
    invite_id: Option<&str>,
) -> Result<(), String> {
    let display_name = state_helpers::current_identity(state)
        .map_err(|_| "identity not set".to_string())?
        .display_name;

    // Gather our profile and mailbox DHT keys + route blob for the invite payload.
    //
    // B4/P3.2 — if our route hasn't landed yet (e.g. "Add friend" within
    // seconds of login), wait up to 5 seconds for its owner to publish it.
    // Sending the request with an empty blob meant the receiver had no
    // inbound path to reply on and the friend handshake silently stalled.
    let (profile_dht_key, mailbox_dht_key) = {
        let node = state.node.read();
        let nh = node.as_ref().ok_or("node not initialized")?;
        (
            nh.profile_dht_key.clone().unwrap_or_default(),
            nh.mailbox_dht_key.clone().unwrap_or_default(),
        )
    };
    const ROUTE_WAIT: std::time::Duration = std::time::Duration::from_secs(5);
    let routes = state_helpers::own_routes(state).ok_or("node not initialized")?;
    let route_blob = tokio::time::timeout(
        ROUTE_WAIT,
        routes.available(rekindle_protocol::own_routes::RouteClass::General),
    )
    .await
    .map_err(|_| "Veilid private route not allocated yet — try again in a moment".to_string())?;

    tracing::info!(
        to = %to,
        route_blob_len = route_blob.len(),
        route_count = route_blob.first().copied().unwrap_or(0),
        "send_friend_request: our route blob info"
    );

    let prekey_bundle = handout_bundle_bytes(state)?;
    let payload = MessagePayload::FriendRequest {
        display_name,
        message: message.to_string(),
        prekey_bundle,
        profile_dht_key,
        route_blob,
        mailbox_dht_key,
        invite_id: invite_id.map(str::to_string),
    };
    super::transport::send_envelope_to_peer(state, pool, to, &payload).await
}

/// Send a friend acceptance to a peer via Veilid.
///
/// Includes our `PreKeyBundle` and (if available) the `SessionInitInfo` from
/// `establish_session()` so the requester can call `respond_to_session()`.
pub async fn send_friend_accept(
    state: &Arc<AppState>,
    pool: &Db,
    to: &str,
    session_init: Option<rekindle_crypto::signal::SessionInitInfo>,
) -> Result<(), String> {
    let prekey_bundle = handout_bundle_bytes(state)?;

    // Gather our profile and mailbox DHT keys + route blob
    let (profile_dht_key, mailbox_dht_key) = {
        let node = state.node.read();
        let nh = node.as_ref().ok_or("node not initialized")?;
        (
            nh.profile_dht_key.clone().unwrap_or_default(),
            nh.mailbox_dht_key.clone().unwrap_or_default(),
        )
    };
    let route_blob = state_helpers::our_route_blob(state).unwrap_or_default();

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
    super::transport::send_envelope_to_peer(state, pool, to, &payload).await
}

/// Send a friend rejection to a peer via Veilid.
pub async fn send_friend_reject(state: &Arc<AppState>, pool: &Db, to: &str) -> Result<(), String> {
    let payload = MessagePayload::FriendReject;
    super::transport::send_envelope_to_peer(state, pool, to, &payload).await
}

/// Send a typing indicator to a peer.
pub async fn send_typing(
    state: &Arc<AppState>,
    pool: &Db,
    to: &str,
    typing: bool,
) -> Result<(), String> {
    let payload = MessagePayload::TypingIndicator { typing };
    super::transport::send_envelope_to_peer(state, pool, to, &payload).await
}

/// Send any payload to a peer, sealed as its
/// [`rekindle_codec::message::envelope::Sealing`] requires. A
/// `Session` payload with no Signal session fails closed.
pub async fn send_to_peer(
    state: &Arc<AppState>,
    pool: &Db,
    to: &str,
    payload: &MessagePayload,
) -> Result<(), String> {
    super::transport::send_envelope_to_peer(state, pool, to, payload).await
}

/// Build a signed `MessageEnvelope` for the given payload and queue it in
/// `pending_messages` for retry by `sync_service`.
///
/// Used by `friends.rs` to always-queue an `Unfriended` message regardless of
/// whether the initial `send_to_peer` succeeded (Veilid `app_message` has
/// no delivery guarantee). The queued entry is cleared when the peer sends an
/// `UnfriendedAck`, or dropped after max retries (20 x 30s).
pub(crate) async fn build_and_queue_envelope(
    state: &Arc<AppState>,
    pool: &Db,
    to: &str,
    payload: &MessagePayload,
) -> Result<(), String> {
    let envelope = super::seal::build_signed_envelope(state, to, payload).await?;
    let envelope_json =
        serde_json::to_string(&envelope).map_err(|e| format!("serialize envelope: {e}"))?;
    queue_pending_message(state, pool, to, &envelope_json).await
}

/// Insert a message into the `pending_messages` table for later retry.
pub(super) async fn queue_pending_message(
    state: &Arc<AppState>,
    pool: &Db,
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
