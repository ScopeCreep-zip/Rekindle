//! DM and group-DM invite payloads.
//!
//! These types travel as `app_call` payloads (fast path) and DHT inbox
//! entries (durable path) from initiator to invitee. For 2-party DMs
//! the MEK is derived deterministically (no `wrapped_mek`). For group
//! DMs the MEK is randomly generated and wrapped per recipient.

use rekindle_types::dm_store::DmParticipant;
use serde::{Deserialize, Serialize};

/// 2-party DM invitation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DmInvite {
    /// SMPL record DHT key.
    pub record_key: String,
    /// 32-byte slot seed for `derive_slot_keypair`.
    pub slot_seed: Vec<u8>,
    /// Display name the initiator wants to be known as.
    pub initiator_pseudonym: String,
    /// Subkey index the initiator writes to (0).
    pub initiator_subkey: u32,
    /// Subkey index the responder writes to (1).
    pub responder_subkey: u32,
}

/// Group DM (3-8 participants) invitation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GroupDmInvite {
    pub record_key: String,
    pub slot_seed: Vec<u8>,
    pub initiator_pseudonym: String,
    /// Full participant list — each entry pairs a pseudonym with their
    /// SMPL subkey and public key.
    pub participants: Vec<DmParticipant>,
    /// MEK material wrapped for THIS specific recipient via ECDH.
    pub wrapped_mek: Vec<u8>,
    /// MEK generation (starts at 0).
    pub mek_generation: u32,
}
