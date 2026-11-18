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

impl ChannelId {
    /// Parse 32 hex characters (the form channel ids take in caches,
    /// records and IPC); `None` for anything else.
    #[must_use]
    pub fn from_hex(hex_str: &str) -> Option<Self> {
        let bytes = hex::decode(hex_str).ok()?;
        Some(Self(bytes.try_into().ok()?))
    }

    /// The 32-character lowercase hex form.
    #[must_use]
    pub fn to_hex(self) -> String {
        hex::encode(self.0)
    }
}

/// 16-byte UUID identifying a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MessageId(pub [u8; 16]);

/// 16-byte UUID identifying a role definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RoleId(pub [u8; 16]);

impl RoleId {
    /// `@everyone`, the role every member holds implicitly.
    pub const EVERYONE: Self = Self([0u8; 16]);

    /// Parse 32 hex characters, the form role ids take at every API
    /// surface (IPC, DTOs, the frontend, SQLite); `None` for anything
    /// else. The full 16 bytes ride everywhere: a shorter form lets two
    /// distinct roles collide (plan E3.2 S9).
    #[must_use]
    pub fn from_hex(hex_str: &str) -> Option<Self> {
        let bytes = hex::decode(hex_str).ok()?;
        Some(Self(bytes.try_into().ok()?))
    }

    /// The 32-character lowercase hex form.
    #[must_use]
    pub fn to_hex(self) -> String {
        hex::encode(self.0)
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
        let ch = ChannelId([1u8; 16]);
        let role = RoleId([1u8; 16]);
        // Identical bytes, different types: the newtypes are the whole
        // point, so assert on the bytes rather than binding and
        // discarding — a discarded binding proves nothing at runtime.
        assert_eq!(ch.0, role.0);
    }

    #[test]
    fn slot_index_range() {
        // Valid range is 0..255 for a 255-member SMPL record
        let slot = SlotIndex(254);
        assert_eq!(slot.0, 254);
    }
}
