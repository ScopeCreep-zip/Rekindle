//! DM and group-DM invite payloads (architecture §27.1 & §27.2).
//!
//! These types travel as `app_call` payloads from initiator to invitee.
//! For 2-party DMs the MEK is derived deterministically (no
//! `wrapped_mek` field), per §27.1. For group DMs, the MEK is randomly
//! generated and wrapped per recipient (§27.2).

use serde::{Deserialize, Serialize};

/// 2-party DM invitation. Sent via `app_call` from Alice → Bob.
/// Bob accepts → derives slot keypair from `slot_seed`, opens the SMPL
/// record, and starts participating. Bob declines → sends a `DmDecline`
/// reply on the same `app_call`; Alice deletes the record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DmInvite {
    /// SMPL record DHT key (Veilid `TypedKey` rendered as string).
    pub record_key: String,
    /// 32-byte slot seed for `derive_slot_keypair` (§8.3 universal flow).
    pub slot_seed: Vec<u8>,
    /// Display name Alice wants to be known as in this DM.
    pub alice_pseudonym: String,
    /// Subkey index Alice writes to (architecture §27.1: 0).
    pub alice_subkey: u32,
    /// Subkey index Bob will write to (architecture §27.1: 1).
    pub bob_subkey: u32,
}

/// Group-DM (3-8 participants) invitation. Same shape as DmInvite but
/// carries the wrapped MEK (random, not ECDH-derived) and the full
/// participant list.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GroupDmInvite {
    pub record_key: String,
    pub slot_seed: Vec<u8>,
    pub initiator_pseudonym: String,
    /// Participant list: each entry pairs a pseudonym with the SMPL
    /// subkey they will write to and their public key (verification).
    pub participants: Vec<GroupDmParticipant>,
    /// MEK material wrapped for *this specific* recipient with their
    /// X25519 identity public key (architecture §27.2).
    pub wrapped_mek: Vec<u8>,
    /// MEK generation; starts at 0 and increments on rotation.
    pub mek_generation: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GroupDmParticipant {
    pub pseudonym: String,
    pub subkey: u32,
    /// Hex-encoded Ed25519 identity public key.
    pub public_key: String,
}
