//! Gossip envelope helpers for transport-layer dedup and forwarding decisions.
//!
//! Types live in `rekindle-types::gossip_payload`. This module provides
//! transport-specific functions that operate on those types using
//! postcard, blake3, and rekindle_utils — dependencies that do not
//! belong in the pure-data types crate.

pub use rekindle_types::gossip_payload::*;

/// Compute a dedup key for a gossip envelope.
///
/// For message notifications: use the message_id.
/// For typing/presence: use a time-bucketed key to collapse rapid updates.
/// For everything else: BLAKE3 hash of the payload bytes.
pub fn dedup_key(envelope: &SignedGossipEnvelope) -> String {
    if let Ok(payload) = postcard::from_bytes::<GossipPayload>(&envelope.payload_bytes) {
        match &payload {
            GossipPayload::MessageNotification { message_id, .. } => {
                return message_id.clone();
            }
            GossipPayload::TypingIndicator { channel_id, .. } => {
                let bucket = rekindle_utils::timestamp_secs() / 5;
                return format!("typing:{channel_id}:{}:{bucket}", envelope.sender_pseudonym);
            }
            GossipPayload::PresenceUpdate { .. } => {
                let bucket = rekindle_utils::timestamp_secs() / 30;
                return format!("presence:{}:{bucket}", envelope.sender_pseudonym);
            }
            GossipPayload::Control(_) => {}
        }
    }
    let hash = blake3::hash(&envelope.payload_bytes);
    hex::encode(&hash.as_bytes()[..16])
}

/// Whether an envelope carries a private payload that should NOT be forwarded
/// to gossip mesh peers.
pub fn is_private(envelope: &SignedGossipEnvelope) -> bool {
    if let Ok(payload) = postcard::from_bytes::<GossipPayload>(&envelope.payload_bytes) {
        matches!(
            payload,
            GossipPayload::Control(
                ControlPayload::JoinAccepted { .. }
                | ControlPayload::JoinRejected { .. }
                | ControlPayload::SlotKeypairGrant { .. }
                | ControlPayload::AdminKeypairGrant { .. }
                | ControlPayload::SyncResponse { .. }
                | ControlPayload::KickedNotification
            )
        )
    } else {
        false
    }
}
