//! Session state types — non-secret metadata persisted as session.json.
//!
//! These types describe the local user's identity, community memberships,
//! DM peer state, and pending friend requests. No secret material — signing
//! keys, Signal sessions, keypair bytes are in the vault, not here.
//!
//! Save/load is in `rekindle-storage::session_meta`.

use std::collections::HashMap;
use serde::{Deserialize, Serialize};

/// Root session metadata for the local user.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionMeta {
    /// The local user's identity. None before `rekindle init`.
    pub identity: Option<SessionIdentity>,

    /// Communities the user has joined, keyed by governance DHT key.
    #[serde(default)]
    pub communities: HashMap<String, CommunityMembership>,

    /// Per-peer DM channel state. Maps peer_public_key → DmPeerLog.
    #[serde(default)]
    pub dm_peers: HashMap<String, DmPeerLog>,

    /// Pending inbound friend requests awaiting user action.
    #[serde(default)]
    pub pending_friend_requests: Vec<PendingFriendRequest>,

    /// Display names of accepted friends, keyed by public key.
    #[serde(default)]
    pub friend_display_names: HashMap<String, String>,

    /// Pending outbound DhtLog keys for sent friend requests.
    /// Maps target_profile_dht_key → outbound_log_key.
    #[serde(default)]
    pub pending_outbound_logs: HashMap<String, String>,

    /// Schema version for forward compatibility.
    #[serde(default = "default_version")]
    pub version: u32,
}

fn default_version() -> u32 { 2 }

impl SessionMeta {
    pub fn pending_request_by_key(&self, pubkey: &str) -> Option<&PendingFriendRequest> {
        self.pending_friend_requests
            .iter()
            .find(|r| r.sender_public_key == pubkey)
    }

    pub fn remove_pending_friend_request(&mut self, pubkey: &str) {
        self.pending_friend_requests
            .retain(|r| r.sender_public_key != pubkey);
    }

    /// Look up a community membership by governance key.
    pub fn community(&self, governance_key: &str) -> Option<&CommunityMembership> {
        self.communities.get(governance_key)
    }

    /// Case-insensitive community name lookup. Returns None if ambiguous.
    pub fn community_by_name(&self, name: &str) -> Option<&CommunityMembership> {
        let lower = name.to_lowercase();
        let matches: Vec<&CommunityMembership> = self
            .communities
            .values()
            .filter(|m| m.community_name.to_lowercase() == lower)
            .collect();
        if matches.len() == 1 { Some(matches[0]) } else { None }
    }
}

/// The local user's cryptographic and network identity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionIdentity {
    pub public_key_hex: String,
    pub display_name: String,
    pub profile_dht_key: String,
    pub mailbox_dht_key: String,
    pub friend_list_dht_key: String,
    pub friend_inbox_key: String,
    pub friend_inbox_keypair_hex: String,
}

/// Per-peer DM channel state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DmPeerLog {
    /// DhtLog spine key I created — I write my outbound messages here.
    pub outbound_log_key: String,
    /// DhtLog spine key the peer created — they write here, I read.
    pub inbound_log_key: String,
}

/// Per-community membership state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunityMembership {
    pub governance_key: String,
    pub pseudonym_key: String,
    pub display_name: String,
    #[serde(default)]
    pub role_ids: Vec<u32>,
    pub registry_key: String,
    pub slot_index: u32,
    pub community_name: String,
    /// Per-channel DhtLog record keys owned by this member.
    /// Maps channel_id (UUID) → DhtLog record key.
    #[serde(default)]
    pub channel_record_keys: HashMap<String, String>,
    /// Channel name → channel UUID resolution map.
    /// Populated at create/join from the governance channel list.
    /// All external interfaces (CLI, TUI, Tauri) send channel names.
    /// All internal operations (MEK cache, record keys, vault queries)
    /// use UUIDs. `resolve_channel()` bridges the two.
    #[serde(default)]
    pub channel_name_to_id: HashMap<String, String>,
    /// Community mailbox DHT key — the community's RPC endpoint.
    #[serde(default)]
    pub community_mailbox_key: String,
    /// Join inbox DHT key (operators only).
    #[serde(default)]
    pub join_inbox_key: String,
    /// Whether this member is an operator (holds the governance keypair).
    #[serde(default)]
    pub is_operator: bool,
    /// Whether the community is currently locked down (no non-operator sends).
    /// Updated by inbound ChannelLockdown gossip. Enforced in messaging send path.
    #[serde(default)]
    pub locked_down: bool,
    #[serde(default)]
    pub joined_at: u64,
}

/// Channel resolution failed — the name or UUID doesn't match any known channel.
#[derive(Debug, Clone)]
pub struct ChannelResolutionError {
    pub requested: String,
    pub known_channels: Vec<String>,
}

impl std::fmt::Display for ChannelResolutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unknown channel '{}' — known channels: {}", self.requested, self.known_channels.join(", "))
    }
}

impl std::error::Error for ChannelResolutionError {}

impl CommunityMembership {
    /// Resolve a channel reference (name OR UUID) to the canonical channel UUID.
    ///
    /// Resolution order:
    /// 1. Exact match in `channel_name_to_id` (user passed a name like "general")
    /// 2. Exact match as a key in `channel_record_keys` (user passed a UUID directly)
    /// 3. Error listing known channel names for remediation
    ///
    /// Every channel operation calls this once at the top. The returned UUID
    /// is used for MEK cache lookups, channel_record_keys, vault queries,
    /// and gossip payloads. No consumer handles name→UUID resolution itself.
    pub fn resolve_channel(&self, channel: &str) -> Result<String, ChannelResolutionError> {
        // Name lookup (CLI sends "general", TUI sends "general")
        if let Some(id) = self.channel_name_to_id.get(channel) {
            return Ok(id.clone());
        }
        // UUID passthrough (internal callers may already have the UUID)
        if self.channel_record_keys.contains_key(channel) {
            return Ok(channel.to_string());
        }
        Err(ChannelResolutionError {
            requested: channel.to_string(),
            known_channels: self.channel_name_to_id.keys().cloned().collect(),
        })
    }
}

/// An inbound friend request awaiting accept/reject.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingFriendRequest {
    pub sender_public_key: String,
    pub display_name: String,
    pub message: String,
    pub profile_dht_key: String,
    pub mailbox_dht_key: String,
    pub prekey_bundle: Vec<u8>,
    pub dm_log_key: String,
    pub dm_log_keypair_hex: String,
    pub received_at: u64,
}
