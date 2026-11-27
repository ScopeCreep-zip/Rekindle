//! Architecture §4.3 — flat description of a community member used by
//! gossip envelopes (`MemberJoined` broadcasts). Mirrors the wire shape that
//! the gossip path serialized as untyped JSON before the v2.0 envelope
//! migration; the typed form here is what `CommunityEnvelope` carries.

use serde::{Deserialize, Serialize};

/// Longest member timeout: Discord's `communication_disabled_until` cap
/// (up to 28 days in the future).
pub const MAX_TIMEOUT_SECONDS: u64 = 28 * 24 * 60 * 60;

/// Member identity + presence snapshot, as broadcast by gossip.
///
/// String IDs match the existing on-the-wire envelope contract used by
/// `rekindle-protocol::dht::community::envelope`. The Cap'n Proto
/// migration (plan: `.claude/plans/community-envelope-capnp-migration.md`,
/// Phase 5) replaces these with typed ID schemas.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberInfo {
    /// Hex-encoded community pseudonym Ed25519 public key.
    pub pseudonym_key: String,
    pub display_name: String,
    pub role_ids: Vec<String>,
    /// `online` / `away` / `busy` / `offline`.
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_blob: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bio: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pronouns: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme_color: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub badges: Vec<String>,
    /// Unix-seconds of the last presence heartbeat from this member.
    #[serde(default)]
    pub last_seen: u64,
}

#[cfg(test)]
#[path = "member_tests.rs"]
mod tests;
