//! Newtype identifiers that prevent mixing up raw byte arrays and strings.
//!
//! Every ID is a distinct type — you can't accidentally pass a ChannelId
//! where a RoleId is expected.

use serde::{Deserialize, Serialize};

/// Community-specific Ed25519 public key identifying a member.
/// Derived via HKDF from the member's master secret + community ID.
/// Unlinkable across communities.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PseudonymKey(pub [u8; 32]);

impl PseudonymKey {
    /// Parse a 64-char hex pseudonym, falling back to all-zero on any
    /// malformed input.
    ///
    /// Lossy by design and named so: the callers are wire-decode paths
    /// where a bad pseudonym must not abort the batch — an all-zero key
    /// matches no member, so the entry is dropped by reader validation
    /// exactly as a forged one would be. Two crates had identical
    /// private copies of this (`hex_to_pseudo_32`).
    #[must_use]
    pub fn from_hex_lossy(hex_str: &str) -> Self {
        Self(
            hex::decode(hex_str)
                .ok()
                .and_then(|b| b.try_into().ok())
                .unwrap_or([0u8; 32]),
        )
    }

    /// Lowercase hex form.
    #[must_use]
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }
}

/// Index into a 255-slot SMPL record. Same index used across
/// governance, registry, and all channel records for a given member.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SlotIndex(pub u32);

/// Opaque community identifier — the DHT key of the governance SMPL record.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CommunityId(pub String);

/// 16-byte UUID identifying a channel within a community.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChannelId(pub [u8; 16]);

/// 16-byte UUID identifying a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MessageId(pub [u8; 16]);

/// 16-byte UUID identifying a role definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RoleId(pub [u8; 16]);

impl RoleId {
    /// Widen a legacy `u32` role id into the 16-byte form.
    ///
    /// Role ids are `u32` at every API surface (IPC, the governance
    /// runtime's `Deps`, the frontend) but 16 bytes in CRDT state. The
    /// mapping is little-endian into the low four bytes, and it lives
    /// here because three crates were each carrying their own copy —
    /// `u32_to_role_id`, `role_id_to_key`, `role_id_to_legacy_u32` —
    /// with a comment asking the next author to keep them identical. A
    /// different padding silently looks up a role that does not exist.
    #[must_use]
    pub fn from_legacy_u32(role_id: u32) -> Self {
        let mut buf = [0u8; 16];
        buf[..4].copy_from_slice(&role_id.to_le_bytes());
        Self(buf)
    }

    /// The inverse of [`Self::from_legacy_u32`].
    #[must_use]
    pub fn to_legacy_u32(self) -> u32 {
        u32::from_le_bytes([self.0[0], self.0[1], self.0[2], self.0[3]])
    }
}

/// 16-byte UUID identifying a category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CategoryId(pub [u8; 16]);

/// 16-byte UUID identifying a thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ThreadId(pub [u8; 16]);

/// 16-byte UUID identifying a scheduled event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EventId(pub [u8; 16]);

// Display impls for debugging
impl std::fmt::Display for PseudonymKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Show first 8 hex chars for readability
        for b in &self.0[..4] {
            write!(f, "{b:02x}")?;
        }
        write!(f, "…")
    }
}

impl std::fmt::Display for CommunityId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pseudonym_key_serde_roundtrip() {
        let key = PseudonymKey([42u8; 32]);
        let json = serde_json::to_string(&key).unwrap();
        let back: PseudonymKey = serde_json::from_str(&json).unwrap();
        assert_eq!(key, back);
    }

    #[test]
    fn distinct_types_not_mixable() {
        // This is a compile-time guarantee — these are different types.
        // If someone tries to pass ChannelId where RoleId is expected, it won't compile.
        let _ch = ChannelId([1u8; 16]);
        let _role = RoleId([1u8; 16]);
        // Even though the bytes are identical, they are not the same type.
    }

    #[test]
    fn slot_index_range() {
        // Valid range is 0..255 for a 255-member SMPL record
        let slot = SlotIndex(254);
        assert_eq!(slot.0, 254);
    }
}
