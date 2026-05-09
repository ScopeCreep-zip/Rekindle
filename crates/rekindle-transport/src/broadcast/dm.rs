//! Peer-to-peer DM broadcast — every DmPayload variant.
//!
//! Each function builds a typed `DmPayload`, serializes it, signs with
//! the identity Ed25519 key, frames with the correct `TypeId`, and sends
//! via `Sender::send_dm` (app_message) or writes to a DhtLog (persistent).
//!
//! DM routing priority:
//! 1. DhtLog (persistent, offline-safe) — if a shared DM log exists for this peer
//! 2. app_message via cached route (real-time, requires peer online)
//! 3. app_message via mailbox route resolution (real-time, requires peer reachable)

use tracing::{debug, info, warn};

use super::dht::channel_log::DhtLog;
use crate::error::{TransportError, Result};
use super::node::TransportNode;
use crate::payload::dm::{DmPayload, GamePresence, dm_type_id, serialize_dm};
use crate::session::Session;

/// Send an encrypted direct message to a peer.
///
/// Prefers DhtLog path (persistent, offline-safe). Falls back to
/// app_message via route (real-time, lost if peer offline).
pub async fn direct_message(
    node: &TransportNode, session: &Session,
    peer_key: &str, body: &[u8], reply_to: Option<Vec<u8>>,
    signing_key: &[u8; 32], dm_log_keypair: Option<veilid_core::KeyPair>,
) -> Result<()> {
    debug!(peer = &peer_key[..12.min(peer_key.len())], body_bytes = body.len(), has_log = dm_log_keypair.is_some(), "dm: direct_message");
    let dm = DmPayload::DirectMessage { body: body.to_vec(), reply_to };

    // DhtLog path
    if let (Some(log_key), Some(kp)) = (session.dm_log_keys.get(peer_key), dm_log_keypair) {
        let dht = node.dht()?;
        let log = DhtLog::open_write(dht.routing_context(), log_key, kp).await?;
        let entry = serde_json::json!({
            "sender_key": session.identity.public_key_hex,
            "body": hex::encode(body),
            "timestamp": rekindle_utils::timestamp_ms(),
            "recipient_key": peer_key,
        });
        let entry_bytes = serde_json::to_vec(&entry)
            .map_err(|e| TransportError::SerializationFailed { reason: e.to_string() })?;
        log.append(&entry_bytes).await?;
        info!(peer = &peer_key[..12.min(peer_key.len())], "DM sent via DhtLog");
        return Ok(());
    }

    // app_message fallback
    send_dm_payload(node, session, peer_key, &dm, signing_key).await
}

/// Send a typing indicator to a peer (always app_message — ephemeral).
pub async fn typing(
    node: &TransportNode, session: &Session,
    peer_key: &str, is_typing: bool, signing_key: &[u8; 32],
) -> Result<()> {
    debug!(peer = &peer_key[..12.min(peer_key.len())], is_typing, "dm: typing");
    let dm = DmPayload::Typing { typing: is_typing };
    send_dm_payload(node, session, peer_key, &dm, signing_key).await
}

/// Send a friend request via DHT inbox write.
///
/// This is a DHT write, not an app_message — the request persists on
/// the DHT until the target reads it. Works when target is offline.
///
/// Delegates to `operations::friend::send_friend_request` which handles
/// the full inbox discovery and write flow.
pub async fn friend_request(
    node: &TransportNode, session: &Session,
    target_profile_key: &str, message: &str, signing_key: &[u8; 32],
) -> Result<()> {
    info!(target = &target_profile_key[..12.min(target_profile_key.len())], "dm: friend_request");
    let _ = crate::operations::friend::send_friend_request(
        node, session, target_profile_key, message, signing_key,
    ).await?;
    Ok(())
}

/// Send a friend accept response via DHT inbox write.
///
/// Creates a shared DM DhtLog and writes the acceptance to the
/// requester's friend inbox.
/// Send a friend accept response via DHT inbox write.
pub async fn friend_accept(
    node: &TransportNode, session: &Session,
    requester_public_key: &str, requester_route_blob: &[u8],
    requester_profile_dht_key: &str, requester_display_name: &str,
) -> Result<crate::operations::friend::FriendAccepted> {
    crate::operations::friend::accept_friend_request(
        node, session, requester_public_key,
        requester_route_blob, requester_profile_dht_key, requester_display_name,
    ).await
}

/// Send a friend reject response via DHT inbox write.
pub async fn friend_reject(
    node: &TransportNode, session: &Session, requester_public_key: &str,
) -> Result<()> {
    crate::operations::friend::reject_friend_request(node, session, requester_public_key).await
}

/// Send a friend request acknowledgement (delivery confirmation).
pub async fn friend_request_ack(
    node: &TransportNode, session: &Session,
    peer_key: &str, signing_key: &[u8; 32],
) -> Result<()> {
    debug!(peer = &peer_key[..12.min(peer_key.len())], "dm: friend_request_ack");
    let dm = DmPayload::FriendRequestAck;
    send_dm_payload(node, session, peer_key, &dm, signing_key).await
}

/// Notify a peer that we've unfriended them.
pub async fn unfriend(
    node: &TransportNode, session: &Session,
    peer_key: &str, signing_key: &[u8; 32],
) -> Result<()> {
    info!(peer = &peer_key[..12.min(peer_key.len())], "dm: unfriend");
    let dm = DmPayload::Unfriend;
    send_dm_payload(node, session, peer_key, &dm, signing_key).await
}

/// Acknowledge an unfriend notification.
pub async fn unfriend_ack(
    node: &TransportNode, session: &Session,
    peer_key: &str, signing_key: &[u8; 32],
) -> Result<()> {
    debug!(peer = &peer_key[..12.min(peer_key.len())], "dm: unfriend_ack");
    let dm = DmPayload::UnfriendAck;
    send_dm_payload(node, session, peer_key, &dm, signing_key).await
}

/// Notify a peer that we rotated our profile DHT key.
pub async fn profile_key_rotated(
    node: &TransportNode, session: &Session,
    peer_key: &str, new_profile_dht_key: &str, signing_key: &[u8; 32],
) -> Result<()> {
    info!(peer = &peer_key[..12.min(peer_key.len())], new_key = &new_profile_dht_key[..12.min(new_profile_dht_key.len())], "dm: profile_key_rotated");
    let dm = DmPayload::ProfileKeyRotated {
        new_profile_dht_key: new_profile_dht_key.into(),
    };
    send_dm_payload(node, session, peer_key, &dm, signing_key).await
}

/// Send a DM presence update to a peer.
pub async fn dm_presence_update(
    node: &TransportNode, session: &Session,
    peer_key: &str, status: u8, game_info: Option<GamePresence>,
    signing_key: &[u8; 32],
) -> Result<()> {
    debug!(peer = &peer_key[..12.min(peer_key.len())], status, "dm: presence_update");
    let dm = DmPayload::PresenceUpdate { status, game_info };
    send_dm_payload(node, session, peer_key, &dm, signing_key).await
}

/// Remove a friend from the DHT friend list and invalidate their route.
pub async fn remove_friend(
    node: &TransportNode, session: &Session, peer_key: &str,
) -> Result<()> {
    info!(peer = &peer_key[..12.min(peer_key.len())], "dm: remove_friend");
    crate::operations::friend::remove_friend(node, session, peer_key).await
}

// ── Internal ───────────────────────────────────────────────────────────

/// Send a DmPayload via app_message to a peer's route.
///
/// Resolves the peer's route from cache or mailbox DHT, then sends
/// a signed, framed app_message.
async fn send_dm_payload(
    node: &TransportNode, session: &Session,
    peer_key: &str, dm: &DmPayload, signing_key: &[u8; 32],
) -> Result<()> {
    let type_id = dm_type_id(dm);
    debug!(peer = &peer_key[..12.min(peer_key.len())], type_id = type_id as u8, "dm: send_dm_payload");
    let route_blob = resolve_peer_route(node, session, peer_key).await?;
    let target = node.import_route(&route_blob)?;
    let payload_bytes = serialize_dm(dm)?;
    let type_id = dm_type_id(dm);

    node.sender().send_dm(
        &target, type_id, signing_key,
        &session.identity.public_key_hex,
        0, None,
        &payload_bytes,
    ).await
}

/// Resolve a peer's route blob from cache or mailbox DHT.
async fn resolve_peer_route(
    node: &TransportNode, session: &Session, peer_key: &str,
) -> Result<Vec<u8>> {
    // Try cache first
    {
        let peers = node.peers();
        let registry = peers.read();
        if let Some(blob) = registry.get_route(peer_key) {
            return Ok(blob.to_vec());
        }
    }

    // Fall back to reading their mailbox
    let mailbox_key = session.pending_friend_requests.iter()
        .find(|r| r.public_key == peer_key)
        .map(|r| r.mailbox_dht_key.clone())
        .unwrap_or_default();

    if mailbox_key.is_empty() {
        warn!(peer = &peer_key[..12.min(peer_key.len())], "dm: no mailbox key for peer");
        return Err(TransportError::NoRoute {
            peer: format!("{}… (no mailbox key)", &peer_key[..12.min(peer_key.len())]),
        });
    }

    let dht = node.dht()?;
    let blob = dht.mailbox().read_peer_route(&mailbox_key).await?
        .ok_or_else(|| TransportError::NoRoute {
            peer: format!("{}… (mailbox empty)", &peer_key[..12.min(peer_key.len())]),
        })?;

    node.peers().write().cache_route(peer_key, blob.clone());
    Ok(blob)
}
