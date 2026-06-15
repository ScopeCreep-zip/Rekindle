//! Session state types — non-secret metadata persisted as session.json.
//!
//! These types describe the local user's identity, community memberships,
//! DM peer state, and pending friend requests. No secret material — signing
//! keys, Signal sessions, keypair bytes are in the vault, not here.
//!
//! Identity fields use typed values from `rekindle-identity` where the
//! value represents an Ed25519 key (`IdentityRoot`) or a governance
//! record key (`GovernanceKey`). DHT infrastructure keys (profile, mailbox,
//! friend inbox, DhtLog spine keys) remain `String` — they're routing
//! addresses, not identity.
//!
//! Save/load is in `rekindle-storage::session_meta`.

use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use rekindle_identity::{IdentityRoot, GovernanceKey};

/// Root session metadata for the local user.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionMeta {
    /// The local user's identity. None before `rekindle init`.
    pub identity: Option<SessionIdentity>,

    /// Communities the user has joined, keyed by governance DHT key string.
    /// The key is the VLD0: governance record key as a string for HashMap
    /// compatibility. Use `resolve_community()` to get a typed `GovernanceKey`.
    #[serde(default)]
    pub communities: HashMap<String, CommunityMembership>,

    /// Per-peer DM channel state, keyed by peer Ed25519 public key hex.
    #[serde(default)]
    pub dm_peers: HashMap<String, DmPeerLog>,

    /// Per-peer SMPL DM conversations, keyed by peer Ed25519 public key hex.
    /// Maps peer_key → record_key for the shared SMPL record.
    #[serde(default)]
    pub dm_smpl_peers: HashMap<String, DmSmplPeer>,

    /// Pending inbound friend requests awaiting user action.
    #[serde(default)]
    pub pending_friend_requests: Vec<PendingFriendRequest>,

    /// Display names of accepted friends, keyed by Ed25519 public key hex.
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

fn default_version() -> u32 { 3 }

impl SessionMeta {
    pub fn pending_request_by_key(&self, pubkey: &IdentityRoot) -> Option<&PendingFriendRequest> {
        let hex = pubkey.to_hex();
        self.pending_friend_requests
            .iter()
            .find(|r| r.sender_public_key == hex)
    }

    pub fn pending_request_by_key_hex(&self, pubkey_hex: &str) -> Option<&PendingFriendRequest> {
        self.pending_friend_requests
            .iter()
            .find(|r| r.sender_public_key == pubkey_hex)
    }

    pub fn remove_pending_friend_request(&mut self, pubkey_hex: &str) {
        self.pending_friend_requests
            .retain(|r| r.sender_public_key != pubkey_hex);
    }

    /// Look up a community membership by governance key string.
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

    /// Resolve a community reference — governance key or name — to the
    /// canonical typed governance key and membership. Tries governance key
    /// lookup first (O(1)), falls back to case-insensitive name search.
    pub fn resolve_community<'a>(&'a self, reference: &str) -> Option<(GovernanceKey, &'a CommunityMembership)> {
        if let Some(m) = self.communities.get(reference) {
            let gov = GovernanceKey::parse(reference).ok()?;
            return Some((gov, m));
        }
        if let Some(m) = self.community_by_name(reference) {
            let gov = GovernanceKey::parse(&m.governance_key).ok()?;
            return Some((gov, m));
        }
        None
    }
}

/// The local user's cryptographic and network identity.
///
/// Persisted to session.json. The runtime identity is `SelfIdentity`
/// from `rekindle-identity`; this struct holds the DHT infrastructure
/// keys that survive across daemon restarts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionIdentity {
    /// Ed25519 identity root. Serializes as 64 hex chars.
    pub public_key: IdentityRoot,
    /// Display name (advisory, not cryptographically signed here).
    pub display_name: String,
    /// Veilid DHT profile record key. Routing only, not identity.
    pub profile_dht_key: String,
    /// Veilid DHT mailbox record key. Routing only, not identity.
    pub mailbox_dht_key: String,
    /// Veilid DHT friend list record key. Routing only.
    pub friend_list_dht_key: String,
    /// Veilid DHT friend inbox record key. Routing only.
    pub friend_inbox_key: String,
    /// Hex-encoded keypair for the friend inbox (secret material).
    pub friend_inbox_keypair_hex: String,
}

/// Per-peer DM channel state (legacy DhtLog path).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DmPeerLog {
    /// DhtLog spine key I created — I write my outbound messages here.
    pub outbound_log_key: String,
    /// DhtLog spine key the peer created — they write here, I read.
    pub inbound_log_key: String,
}

/// Per-peer SMPL DM record mapping.
/// Populated when a DM conversation is created or accepted.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DmSmplPeer {
    /// The shared SMPL record key both parties read/write.
    pub record_key: String,
    /// Whether this is a group DM.
    pub is_group: bool,
}

/// Per-community membership state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunityMembership {
    /// Veilid DHT governance record key string. Parse to `GovernanceKey`
    /// for typed operations via `GovernanceKey::parse()`.
    pub governance_key: String,
    /// Ed25519 pseudonym public key, 64 hex chars. Community-scoped.
    pub pseudonym_key: String,
    pub display_name: String,
    #[serde(default)]
    pub role_ids: Vec<u32>,
    pub registry_key: String,
    pub slot_index: u32,
    pub community_name: String,
    #[serde(default)]
    pub channel_record_keys: HashMap<String, String>,
    #[serde(default)]
    pub channel_name_to_id: HashMap<String, String>,
    /// Channel UUID → slowmode_seconds. Populated at join/create from governance channel list.
    #[serde(default)]
    pub channel_slowmode: HashMap<String, u32>,
    #[serde(default)]
    pub community_mailbox_key: String,
    #[serde(default)]
    pub join_inbox_key: String,
    #[serde(default)]
    pub is_operator: bool,
    #[serde(default)]
    pub locked_down: bool,
    #[serde(default)]
    pub joined_at: u64,
    /// Per-member per-channel last processed DhtLog sequence.
    /// Key: "member_pseudonym:channel_id". Value: last seen sequence.
    /// Used by slow-path catch-up to read only new entries.
    #[serde(default)]
    pub last_seen_seqs: HashMap<String, u64>,
    /// Shared slot seed for deriving SMPL member slot keypairs.
    /// Every member derives every slot's Ed25519 keypair from this seed.
    /// Distributed via CommunityMetadata.slot_seed_hex at join time.
    #[serde(default)]
    pub slot_seed: Option<String>,
    /// Per-community Lamport counter for causal ordering across channels.
    #[serde(default)]
    pub lamport_counter: u64,
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
    pub fn resolve_channel(&self, channel: &str) -> Result<String, ChannelResolutionError> {
        if let Some(id) = self.channel_name_to_id.get(channel) {
            return Ok(id.clone());
        }
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
    /// Sender's Ed25519 public key, 64 hex chars.
    pub sender_public_key: String,
    pub display_name: String,
    pub message: String,
    /// Sender's Veilid DHT profile record key. Routing only.
    pub profile_dht_key: String,
    /// Sender's Veilid DHT mailbox record key. Routing only.
    pub mailbox_dht_key: String,
    pub prekey_bundle: Vec<u8>,
    pub dm_log_key: String,
    /// Hex-encoded keypair for the shared DM DhtLog (secret material).
    pub dm_log_keypair_hex: String,
    pub received_at: u64,
}
