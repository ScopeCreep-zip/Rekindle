//! Application-level session state.
//!
//! [`Session`] holds the local identity, community memberships, DM log keys
//! and pending friend requests. It is an in-memory projection of the
//! unlocked identity's stores (plan D1): the daemon builds it at unlock
//! from the identity's database and vault (`rekindle_node::host::session`)
//! and writes every change to those stores, not to this struct alone. It
//! is `None` while locked.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ── Session ─────────────────────────────────────────────────────────────

/// Root session state for the local user.
#[derive(Debug, Clone)]
pub struct Session {
    /// The local user's identity.
    pub identity: SessionIdentity,

    /// Communities the user has joined, keyed by governance DHT key.
    pub communities: HashMap<String, CommunityMembership>,

    /// Per-peer DM DhtLog spine keys. Maps peer_public_key → DhtLog spine key.
    /// Created during friend accept. Both peers read/write their shared log.
    pub dm_log_keys: HashMap<String, String>,

    /// Pending inbound friend requests awaiting user action.
    pub pending_friend_requests: Vec<PendingFriendRequest>,
}

impl Session {
    /// Create a new session from a freshly created identity.
    pub fn new(identity: SessionIdentity) -> Self {
        Self {
            identity,
            communities: HashMap::new(),
            dm_log_keys: HashMap::new(),
            pending_friend_requests: Vec::new(),
        }
    }

    /// Add a community membership. Overwrites if the governance key already exists.
    pub fn join_community(&mut self, membership: CommunityMembership) {
        self.communities
            .insert(membership.governance_key.clone(), membership);
    }

    /// Remove a community membership by governance key.
    pub fn leave_community(&mut self, governance_key: &str) {
        self.communities.remove(governance_key);
    }

    /// Look up a community membership by governance key.
    pub fn community(&self, governance_key: &str) -> Option<&CommunityMembership> {
        self.communities.get(governance_key)
    }

    /// Look up a community membership by community name (case-insensitive).
    ///
    /// Returns `None` if zero or more than one community matches. The CLI
    /// should use governance key for unambiguous lookups; this is a
    /// convenience for user-facing name resolution.
    pub fn community_by_name(&self, name: &str) -> Option<&CommunityMembership> {
        let lower = name.to_lowercase();
        let matches: Vec<&CommunityMembership> = self
            .communities
            .values()
            .filter(|m| m.community_name.to_lowercase() == lower)
            .collect();
        if matches.len() == 1 {
            Some(matches[0])
        } else {
            None
        }
    }
}

// ── Identity ────────────────────────────────────────────────────────────

/// The local user's cryptographic and network identity.
#[derive(Debug, Clone)]
pub struct SessionIdentity {
    /// Ed25519 public key (hex-encoded, 64 chars).
    pub public_key_hex: String,

    /// User's display name.
    pub display_name: String,

    /// Profile DHT record key.
    pub profile_dht_key: String,

    /// Mailbox DHT record key (deterministic from identity keypair).
    pub mailbox_dht_key: String,

    /// Friend list DHT record key.
    pub friend_list_dht_key: String,

    /// Friend inbox DHT record key (DFLT(32), published keypair).
    /// Other users write friend requests here. Our daemon polls it.
    pub friend_inbox_key: String,

    /// Hex-encoded keypair for the friend inbox. Published in our
    /// profile so anyone can open the inbox for writing.
    pub friend_inbox_keypair_hex: String,

    /// The profile record's owner keypair, in its string form (as the
    /// vault stores it), for re-opening the record writable.
    pub profile_owner_keypair: Option<String>,

    /// The friend list record's owner keypair, in its string form.
    pub friend_list_owner_keypair: Option<String>,
}

// ── Community membership ────────────────────────────────────────────────

/// Per-community membership state.
#[derive(Debug, Clone)]
pub struct CommunityMembership {
    /// Community governance DHT key.
    pub governance_key: String,

    /// Our pseudonym public key for this community (hex).
    pub pseudonym_key: String,

    /// Our role IDs in this community.
    pub role_ids: Vec<u32>,

    /// Member registry DHT key for this community.
    pub registry_key: String,

    /// Our slot index in the member registry.
    pub slot_index: u32,

    /// Cached community name (from governance metadata).
    pub community_name: String,

    /// Slot seed bytes for deriving our SMPL writer keypair (the vault's
    /// `SlotSeed`).
    pub slot_seed: Option<[u8; 32]>,

    /// Per-channel message record keys owned by this member.
    /// Maps channel_id → DHT record key (DFLT(1), member-owned): the
    /// `channels.message_record_key` column.
    pub channel_record_keys: HashMap<String, String>,

    // ── v2.0 governance-runtime fields ──
    //
    // The daemon's `GovernanceRuntimeDeps` adapter reports these across
    // the Schwarzschild boundary as `governance_runtime::CommunityMembership`.
    /// Plate Gate segment hosting our slot (architecture §15).
    /// `None` = unknown/legacy; `0` = genesis segment. The global slot
    /// used for keypair derivation is `segment * SLOTS_PER_SEGMENT +
    /// slot_index`, so for segment 0 the two are equal.
    pub segment_index: Option<u32>,

    /// Governance CRDT Lamport counter for this community.
    ///
    /// NOT the gossip mesh clock (`GossipMesh::clock`) — that one orders
    /// epidemic broadcast, this one orders governance entries for the
    /// CRDT merge. Two clocks, deliberately separate.
    pub lamport_counter: u64,

    /// Highest community-level MEK generation we have seen.
    pub mek_generation: u64,
}

// ── Pending friend requests ─────────────────────────────────────────────

/// An inbound friend request awaiting the user's accept/reject decision.
///
/// Created when a `DmPayload::FriendRequest` is received via the
/// InboundHandler. Stores all the data needed to call
/// `operations::friend::accept_friend_request` when the user decides.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingFriendRequest {
    /// Requester's Ed25519 public key (hex).
    pub public_key: String,

    /// Requester's display name.
    pub display_name: String,

    /// Message attached to the request.
    pub message: String,

    /// Requester's profile DHT record key (for watching presence after accept).
    pub profile_dht_key: String,

    /// Requester's route blob (for sending the accept response).
    pub route_blob: Vec<u8>,

    /// Requester's mailbox DHT key.
    pub mailbox_dht_key: String,

    /// Requester's prekey bundle (for Signal session establishment).
    pub prekey_bundle: Vec<u8>,

    /// Invite ID if the request came through an invite link.
    pub invite_id: Option<String>,

    /// Epoch ms when the request was received.
    pub received_at: u64,
}

impl Session {
    /// Record an inbound friend request. Deduplicates by public key —
    /// if a request from the same peer already exists, it's replaced
    /// (the peer may have re-sent with updated route/prekey data).
    pub fn add_pending_friend_request(&mut self, request: PendingFriendRequest) {
        self.pending_friend_requests
            .retain(|r| r.public_key != request.public_key);
        self.pending_friend_requests.push(request);
    }

    /// Look up a pending friend request by the requester's public key.
    pub fn pending_request_by_key(&self, public_key: &str) -> Option<&PendingFriendRequest> {
        self.pending_friend_requests
            .iter()
            .find(|r| r.public_key == public_key)
    }

    /// Remove a pending friend request (after accept or reject).
    pub fn remove_pending_friend_request(&mut self, public_key: &str) {
        self.pending_friend_requests
            .retain(|r| r.public_key != public_key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_identity() -> SessionIdentity {
        SessionIdentity {
            public_key_hex: "abcd1234".repeat(8),
            display_name: "alice".into(),
            profile_dht_key: "VLD0:profile:key".into(),
            mailbox_dht_key: "VLD0:mailbox:key".into(),
            friend_list_dht_key: "VLD0:friends:key".into(),
            friend_inbox_key: "VLD0:friend-inbox:key".into(),
            friend_inbox_keypair_hex: String::new(),
            profile_owner_keypair: None,
            friend_list_owner_keypair: None,
        }
    }

    fn test_membership() -> CommunityMembership {
        CommunityMembership {
            governance_key: "VLD0:gov:key".into(),
            pseudonym_key: "pseudonym_hex".into(),
            role_ids: vec![1, 2],
            registry_key: "VLD0:reg:key".into(),
            slot_index: 3,
            community_name: "dev-team".into(),
            slot_seed: None,
            channel_record_keys: HashMap::new(),
            segment_index: Some(0),
            lamport_counter: 7,
            mek_generation: 2,
        }
    }

    #[test]
    fn community_by_name_case_insensitive() {
        let mut session = Session::new(test_identity());
        session.join_community(test_membership());

        assert!(session.community_by_name("dev-team").is_some());
        assert!(session.community_by_name("Dev-Team").is_some());
        assert!(session.community_by_name("DEV-TEAM").is_some());
        assert!(session.community_by_name("nonexistent").is_none());
    }

    #[test]
    fn community_by_name_ambiguous_returns_none() {
        let mut session = Session::new(test_identity());

        let mut m1 = test_membership();
        m1.governance_key = "gov1".into();
        m1.community_name = "shared-name".into();
        session.join_community(m1);

        let mut m2 = test_membership();
        m2.governance_key = "gov2".into();
        m2.community_name = "shared-name".into();
        session.join_community(m2);

        // Two communities with the same name — ambiguous
        assert!(session.community_by_name("shared-name").is_none());
    }

    #[test]
    fn leave_community_removes() {
        let mut session = Session::new(test_identity());
        session.join_community(test_membership());
        assert_eq!(session.communities.len(), 1);

        session.leave_community("VLD0:gov:key");
        assert_eq!(session.communities.len(), 0);
    }
}
