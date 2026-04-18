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
