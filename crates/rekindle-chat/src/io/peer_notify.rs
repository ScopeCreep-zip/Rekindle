//! Peer-to-peer ephemeral notification delivery.
//!
//! Typing indicators, read receipts, presence pings, friend request acks,
//! unfriend notifications — any real-time notification to a specific peer
//! that is acceptable to lose if the peer is offline.
//!
//! Every outbound notification is:
//! 1. Serialized (postcard) as the DmPayload
//! 2. TypeId determined from the payload variant
//! 3. Wrapped in a SignedEnvelope (Ed25519-signed with identity key)
//! 4. Dispatched via transport.send_to_peer
//!
//! These are ephemeral — lost if the peer is offline. Durable messages
//! use DHT writes (write_record / write_and_notify), not this path.

use std::time::Instant;

use rekindle_types::dm_payload::DmPayload;

use super::{Confirm, PlatformIO, SendReceipt};
use crate::crypto::envelope::SignedEnvelope;
use crate::ChatError;

/// Map DmPayload variant to its TypeId byte for wire framing.
fn payload_type_id(payload: &DmPayload) -> u8 {
    match payload {
        DmPayload::Typing { .. } => 1,
        DmPayload::FriendRequestAck => 2,
        DmPayload::Unfriend => 3,
        DmPayload::UnfriendAck => 4,
        DmPayload::ProfileKeyRotated { .. } => 5,
        DmPayload::PresenceUpdate { .. } => 6,
    }
}

impl PlatformIO {
    /// Send an ephemeral notification to a peer.
    ///
    /// The payload is serialized, signed with the identity key, and sent
    /// as an app_message. Lost if the peer is offline — use `write_record`
    /// for durable delivery.
    ///
    /// `confirm`: typically `Confirm::None` for typing/presence (ephemeral,
    /// high frequency, loss acceptable), `Confirm::Accepted` for unfriend/ack
    /// (low frequency, should confirm transport accepted the bytes).
    pub async fn send_peer_notification(
        &self,
        peer_key: &str,
        payload: DmPayload,
        confirm: Confirm,
    ) -> Result<SendReceipt, ChatError> {
        let start = Instant::now();
        let type_id = payload_type_id(&payload);

        let payload_bytes = postcard::to_stdvec(&payload)
            .map_err(|e| ChatError::Serialization(format!(
                "peer notification (type 0x{type_id:02x}): {e}"
            )))?;

        let signing_seed = self.require_signing_key()?;
        let wire = SignedEnvelope::build(type_id, &signing_seed, &payload_bytes)?;

        self.transport()
            .send_to_peer(peer_key, &wire)
            .await
            .map_err(|e| {
                tracing::debug!(
                    peer = &peer_key[..12.min(peer_key.len())],
                    type_id,
                    error = %e,
                    "peer notification delivery failed"
                );
                ChatError::Transport(e)
            })?;

        let achieved = if confirm == Confirm::None {
            Confirm::None
        } else {
            Confirm::Accepted
        };

        Ok(SendReceipt {
            peer_key: peer_key.to_string(),
            confirmed: achieved,
            elapsed: start.elapsed(),
        })
    }
}
