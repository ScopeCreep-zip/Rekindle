//! Wire envelope format for all community P2P traffic.
//!
//! Replaces the request/response model (`CommunityRequest`/`CommunityResponse`/`CommunityBroadcast`)
//! with unidirectional envelopes sent via `app_message` (fire-and-forget).

use serde::{Deserialize, Serialize};

mod control;
mod payloads;
mod sign;

pub use control::ControlPayload;
pub use payloads::{
    MekTransferAckPayload, MekTransferPayload, VideoFragmentPayload, VideoParityFragmentPayload,
};
pub use sign::{sign_envelope, verify_envelope};

/// Sent via `app_message` (fire-and-forget) -- NOT `app_call` (request/response).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum CommunityEnvelope {
    /// Gossip notification that a new message exists in a channel SMPL record.
    ///
    /// Chiral Network model: gossip carries the notification (cargo manifest),
    /// not the cargo (ciphertext). Recipients fetch the actual MEK-encrypted
    /// content from the sender's SMPL subkey via `get_dht_value`.
    ///
    /// This ensures ciphertext exists only on DHT storage nodes (5 replicas),
    /// not across the entire gossip fan-out graph (50-100+ relay nodes).
    MessageNotification {
        channel_id: String,
        message_id: String,
        author_pseudonym: String,
        /// Sender's SMPL subkey index — where to fetch the ciphertext.
        subkey_index: u32,
        /// Lamport logical timestamp for causal ordering.
        lamport_ts: u64,
        /// Per-sender, per-channel sequence number for gap detection.
        sequence: u64,
        /// blake3 hash of the MEK-encrypted ciphertext, for integrity
        /// verification after DHT fetch. Ensures the fetched value matches
        /// what the sender wrote.
        content_hash: String,
        timestamp: u64,
    },
    /// A control operation (channel/role/invite/event management, moderation, etc.).
    Control(ControlPayload),
    /// Presence update from a member.
    PresenceUpdate {
        pseudonym_key: String,
        status: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        game_info: Option<PresenceGameInfo>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        route_blob: Option<Vec<u8>>,
    },
    /// Typing indicator (ephemeral, not stored).
    TypingIndicator {
        channel_id: String,
        pseudonym_key: String,
    },
    /// Watch Relay (architecture §14.3 / §11.7): a member with an active
    /// Veilid `watch_dht_values` slot relays a `ValueChange` notification
    /// to peers via gossip, so members without watch slots still learn
    /// when a record's subkey changes. The receiver is expected to
    /// `get_dht_value` to fetch the new value (we deliberately do NOT
    /// carry ciphertext over gossip — same Chiral Network principle as
    /// `MessageNotification`).
    WatchRelay {
        /// Hex-encoded Veilid record key whose subkey changed.
        record_key: String,
        /// Subkey index that changed.
        subkey: u32,
        /// blake3 hash of the new value (integrity check after fetch).
        content_hash: String,
        /// Sender's pseudonym (for permission/audit).
        observer_pseudonym: String,
    },
}

impl CommunityEnvelope {
    /// Is this envelope addressed to **one** member rather than the
    /// community?
    ///
    /// Directed payloads must never be gossip-forwarded. Two things go
    /// wrong if they are: a payload wrapped for a single recipient is
    /// amplified to every peer within the TTL for no one's benefit, and
    /// §10.6 requires that channel media stay on the roster it was sent
    /// to rather than fanning out epidemically.
    ///
    /// This is the single definition. There were two, and they
    /// disagreed: `rekindle-transport`'s postcard
    /// `SignedGossipEnvelope::is_private` listed six variants while
    /// `src-tauri`'s `is_private_control_payload` listed four, omitting
    /// `JoinRejected` and `KickedNotification` — so a desktop node
    /// re-broadcast two payloads a daemon node would not. The union is
    /// the correct answer, plus the media plane, which was previously
    /// guarded separately and only on one track.
    #[must_use]
    pub fn is_directed(&self) -> bool {
        let Self::Control(payload) = self else {
            return false;
        };
        matches!(
            payload,
            // Wrapped or granted to a single recipient.
            ControlPayload::JoinAccepted { .. }
                | ControlPayload::JoinRejected { .. }
                | ControlPayload::SlotKeypairGrant { .. }
                | ControlPayload::AdminKeypairGrant { .. }
                | ControlPayload::MekTransfer(_)
                | ControlPayload::MekTransferAck(_)
                | ControlPayload::KickedNotification
                // Answers to one peer's question.
                | ControlPayload::SyncResponse { .. }
                | ControlPayload::BootstrapResponse { .. }
                // Media plane — §10.6. Directed to a call or channel
                // roster; flooding it would multiply a video stream by
                // the fan-out degree at every hop.
                | ControlPayload::VideoFragment(_)
                | ControlPayload::VideoParityFragment(_)
                | ControlPayload::FrameAck { .. }
                | ControlPayload::KeyframeRequest { .. }
                | ControlPayload::BandwidthEstimate { .. }
                | ControlPayload::MediaCapabilities { .. }
                | ControlPayload::TopologyChange { .. }
                | ControlPayload::AttachmentChunk { .. }
                | ControlPayload::MultiAttachmentChunk { .. }
                | ControlPayload::RequestAttachment { .. }
        )
    }
}

/// Game information for community presence.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresenceGameInfo {
    pub game_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_id: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_seconds: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_address: Option<String>,
}

/// A participant entry in a voice roster broadcast.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceRosterEntry {
    pub pseudonym_key: String,
    pub route_blob: Vec<u8>,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub deafened: bool,
    /// Display name as known to the roster broadcaster — identity
    /// rides the handshake (SimpleX `x.grp.mem.info` pattern) so the
    /// roster UI never depends on registry-scan timing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

/// Signed wrapper: sender_pseudonym + serialized envelope + Ed25519
/// signature, for SMPL/gossip payloads.
///
/// Declared once, in `rekindle-codec` — the Tier 3 serialization crate
/// whose own module doc already claims signed-envelope construction as
/// its scope. This crate carried a field-identical twin (same five
/// fields, same camelCase, same `ttl` default of 5), so gossip ended up
/// importing both: `mesh_broadcast.rs` and `deps.rs` took this one,
/// `broadcast.rs` took codec's. `wire_tests.rs` proved them
/// byte-identical before they were folded together.
///
/// Not to be confused with the DM transport envelope
/// (`rekindle-transport`'s `SignedPayload`); per
/// `docs/contributor/dm-envelope-interop.md` that is a different job and
/// converges separately.
pub use rekindle_codec::envelope::SignedEnvelope;

/// A single onboarding answer.
/// Re-exported from Tier 1 rather than declared here. Two declarations
/// of this name existed with *different fields*; the vocabulary tier is
/// the one home for the shape, and this crate's Cap'n Proto codec reads
/// and writes exactly it.
pub use rekindle_types::presence::OnboardingAnswer;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod wire_tests;
