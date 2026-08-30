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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnboardingAnswer {
    pub question_id: String,
    pub selected_options: Vec<String>,
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod wire_tests;
