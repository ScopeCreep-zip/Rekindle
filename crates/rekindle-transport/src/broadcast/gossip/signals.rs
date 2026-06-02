//! Top-level `GossipPayload` signal broadcasts (messages, typing, presence).

use parking_lot::RwLock;
use tracing::trace;

use super::super::node::TransportNode;
use super::super::send::BroadcastReport;
use super::super::OutboundRateLimiter;
use super::{build_sign_send, MeshMap};
use crate::payload::gossip::GossipPayload;

/// Minimum interval between typing broadcasts per (community, channel).
const TYPING_RATE_LIMIT: std::time::Duration = std::time::Duration::from_secs(3);
/// Minimum interval between presence broadcasts per sender.
const PRESENCE_RATE_LIMIT: std::time::Duration = std::time::Duration::from_secs(30);

/// Broadcast a `MessageNotification` after appending to a channel DhtLog.
pub async fn message_notification(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    channel_id: &str,
    message_id: &str,
    author_pseudonym: &str,
    sequence: u64,
    content_hash: &str,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    let payload = GossipPayload::MessageNotification {
        channel_id: channel_id.into(),
        message_id: message_id.into(),
        author_pseudonym: author_pseudonym.into(),
        subkey_index: 0,
        lamport_ts: 0,
        sequence,
        content_hash: content_hash.into(),
        timestamp: rekindle_utils::timestamp_ms(),
    };
    build_sign_send(
        node,
        meshes,
        community_id,
        author_pseudonym,
        signing_key,
        payload,
    )
    .await
}

/// Broadcast a `TypingIndicator`. Rate-limited: max 1 per 3s per (community, channel).
pub async fn typing_indicator(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    rate_limiter: &RwLock<OutboundRateLimiter>,
    community_id: &str,
    channel_id: &str,
    pseudonym_key: &str,
    signing_key: &[u8; 32],
) -> Option<BroadcastReport> {
    let key = format!("{community_id}:typing:{channel_id}");
    if !rate_limiter.write().check(&key, TYPING_RATE_LIMIT) {
        trace!(community_id, channel_id, "typing broadcast rate-limited");
        return None;
    }
    let payload = GossipPayload::TypingIndicator {
        channel_id: channel_id.into(),
        pseudonym_key: pseudonym_key.into(),
    };
    Some(
        build_sign_send(
            node,
            meshes,
            community_id,
            pseudonym_key,
            signing_key,
            payload,
        )
        .await,
    )
}

/// Broadcast a `PresenceUpdate`. Rate-limited: max 1 per 30s per sender.
pub async fn presence_update(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    rate_limiter: &RwLock<OutboundRateLimiter>,
    community_id: &str,
    pseudonym_key: &str,
    status: &str,
    game_name: Option<&str>,
    game_id: Option<u32>,
    elapsed_seconds: Option<u32>,
    server_address: Option<&str>,
    route_blob: Option<Vec<u8>>,
    signing_key: &[u8; 32],
) -> Option<BroadcastReport> {
    let key = format!("{community_id}:presence:{pseudonym_key}");
    if !rate_limiter.write().check(&key, PRESENCE_RATE_LIMIT) {
        trace!(community_id, "presence broadcast rate-limited");
        return None;
    }
    let payload = GossipPayload::PresenceUpdate {
        pseudonym_key: pseudonym_key.into(),
        status: status.into(),
        game_name: game_name.map(String::from),
        game_id,
        elapsed_seconds,
        server_address: server_address.map(String::from),
        route_blob,
    };
    Some(
        build_sign_send(
            node,
            meshes,
            community_id,
            pseudonym_key,
            signing_key,
            payload,
        )
        .await,
    )
}
