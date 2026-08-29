//! Wire format for a governance SMPL subkey value: the signed envelope
//! wrapping one author's batch of [`super::entry::GovernanceEntry`]
//! writes.

use serde::{Deserialize, Serialize};

use crate::id::PseudonymKey;

use super::entry::GovernanceEntry;

/// Wire format for a governance SMPL subkey value.
///
/// Wraps governance entries with the author's community pseudonym so the
/// CRDT merge engine knows who wrote each subkey without relying on the
/// SMPL slot keypair (which is community-shared by design — see
/// `rekindle_secrets::derive::derive_slot_keypair`).
///
/// Architecture §26 W26 line 4140 — `signature` is an Ed25519 signature
/// by the author's pseudonym secret over [`signing_bytes`]. Any reader
/// MUST verify the signature against `author_pseudonym` (which is itself
/// the Ed25519 public key) before applying the entries to local state;
/// otherwise any community member could impersonate any other by writing
/// a forged payload to any subkey (the slot keypair authentication on
/// `set_dht_value` proves only "some member wrote this," not which one).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GovernanceSubkeyPayload {
    pub author_pseudonym: PseudonymKey,
    pub entries: Vec<GovernanceEntry>,
    /// VLD0 key of this author's *next* governance overflow record in the
    /// chain, or `None` when these entries fit a single subkey. Set when one
    /// author's compacted entry log exceeds the SMPL per-subkey cap and spills
    /// into a member-owned overflow record (architecture §"Follow
    /// GovernanceOverflow pointers", line 1609; `overflow_next` header, line
    /// 305). Readers MUST open the pointed-at record, verify each payload
    /// against THIS `author_pseudonym`, and merge its entries before running
    /// the CRDT merge, otherwise spilled state silently vanishes from the
    /// merged view. Authenticated by [`signing_bytes`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overflow_next: Option<String>,
    /// 64-byte Ed25519 signature over [`signing_bytes`]. Empty `Vec` for
    /// pre-signature payloads in disk fixtures or in-flight legacy
    /// rows; readers treat empty signatures as authentication failure
    /// once SCHEMA_VERSION 59 ships.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signature: Vec<u8>,
}

impl GovernanceSubkeyPayload {
    /// Canonical bytes signed by the author. Receivers reproduce these
    /// bytes from the deserialized payload and verify against
    /// `author_pseudonym`. Including the entry count and a domain tag
    /// stops cross-protocol forgeries (a signature for a presence write
    /// can't be replayed as a governance write).
    ///
    /// The `overflow_next` pointer is bound into the signature so a rogue
    /// member cannot rewrite an author's chain to redirect readers at a
    /// record they control. When the pointer is absent (`None`) it appends
    /// nothing, so a non-overflow payload's signed bytes are exactly the
    /// canonical governance form — every governance subkey shares the one
    /// `rekindle-gov-subkey-v1` domain tag.
    pub fn signing_bytes(&self) -> Vec<u8> {
        let entries_json = serde_json::to_vec(&self.entries).unwrap_or_default();
        let next = self.overflow_next.as_deref().unwrap_or("");
        let mut out = Vec::with_capacity(
            b"rekindle-gov-subkey-v1".len() + 32 + 8 + entries_json.len() + next.len(),
        );
        out.extend_from_slice(b"rekindle-gov-subkey-v1");
        out.extend_from_slice(&self.author_pseudonym.0);
        out.extend_from_slice(&(self.entries.len() as u64).to_le_bytes());
        out.extend_from_slice(&entries_json);
        out.extend_from_slice(next.as_bytes());
        out
    }
}
