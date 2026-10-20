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
    /// The prekey maintenance published to our prekey record (plan E2.4):
    /// the head after an SPK and PQ-LR rotation, or a one-time slot.
    /// Triggered by: the "prekey maintenance" task and the slot refill
    /// after a first contact consumed one.
    PrekeyRecordPublished {
        kind: PrekeyPublishKind,
        /// The record subkey written (0: the head; 1..=100: a slot).
        subkey: u32,
    },
}

/// What a [`CryptoEvent::PrekeyRecordPublished`] wrote.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PrekeyPublishKind {
    /// A new signed prekey and PQ last-resort key (the head).
    SignedRotated,
    /// A one-time slot, refilled or topped up.
    OneTimeRefilled,
}
