//! Cryptographic key events — MEK rotation, request, transfer.
//!
//! MEK (Message Encryption Key) events signal that channel encryption
//! keys have changed. Consumers must re-fetch from the vault and
//! re-cache before decrypting new messages.

use serde::{Deserialize, Serialize};

/// Cryptographic key lifecycle events.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Phase 3a — a PQXDH bundle component was published to our profile
    /// record. Emitted by the subkey-5 write path (Phase 3b wires this).
    /// Phase 3a declares the variant so the cross-tier `EventDedup` hash
    /// already knows the shape when 3b starts firing it.
    PqBundlePublished {
        /// DHT subkey number written (5 = canonical PqPreKeyBundle).
        subkey: u16,
        /// Which component of the bundle was rotated.
        kind: PqBundleKind,
    },
}

/// Identifies which PQ bundle component was rotated in a
/// [`CryptoEvent::PqBundlePublished`] event.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PqBundleKind {
    /// Long-rotation last-resort ML-KEM key.
    LastResort,
    /// Batch of one-time ML-KEM keys (consumed individually by initiators).
    OneTimeBatch,
}
