//! Cryptographic key events — MEK rotation, request, transfer.
//!
//! MEK (Message Encryption Key) events signal that channel encryption
//! keys have changed. Consumers must re-fetch from the vault and
//! re-cache before decrypting new messages.

use serde::{Deserialize, Serialize};

/// Cryptographic key lifecycle events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum CryptoEvent {
    /// A channel MEK was rotated (new generation available).
    /// Triggered by: gossip `ControlPayload::MekRotated`, DHT watch on registry MEK vault.
    MekRotated {
        community: String,
        channel: Option<String>,
        generation: u64,
        rotator_pseudonym: Option<String>,
    },
    /// A member is requesting a MEK they don't have (missed rotation).
    /// Triggered by: gossip `ControlPayload::RequestMek`.
    MekRequested {
        community: String,
        channel: String,
        needed_generation: u64,
        requester_pseudonym: String,
    },
    /// A MEK was transferred to us (wrapped for our pseudonym).
    /// Triggered by: gossip `ControlPayload::MekTransfer`.
    MekTransferred {
        community: String,
        channel: Option<String>,
        generation: u64,
        sender_pseudonym: String,
    },
    /// An admin keypair was granted to us (operator delegation).
    /// Triggered by: gossip `ControlPayload::AdminKeypairGrant`.
    AdminKeypairGranted { community: String },
    /// A slot keypair was granted to us (per-channel write access).
    /// Triggered by: gossip `ControlPayload::SlotKeypairGrant`.
    SlotKeypairGranted {
        community: String,
        slot_index: u32,
        segment_index: u32,
    },
    /// One publish to our prekey record (plan E2.4): the head after an SPK
    /// and PQ-LR rotation, the one-time slots refilled or topped up, or
    /// both. One event per publish call, so a new identity's first set is
    /// one event, not 101.
    /// Triggered by: the unlock's first set and reconciliation, the
    /// "prekey maintenance" task and the slot refill after a first contact
    /// consumed one.
    PrekeyRecordPublished {
        /// The head (subkey 0) was written.
        head: bool,
        /// The one-time slots written (1..=100), ascending.
        slots: Vec<u32>,
    },
}
