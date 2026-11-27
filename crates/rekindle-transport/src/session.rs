//! Application-level session state.
//!
//! [`Session`] holds the local identity, community memberships and friends.
//! It is an in-memory projection of the
//! unlocked identity's stores (plan D1): the daemon builds it at unlock
//! from the identity's database and vault (`rekindle_node::host::session`)
//! and writes every change to those stores, not to this struct alone. It
//! is `None` while locked.

use std::collections::HashMap;

use rekindle_secrets::friend_presence::PresenceKey;
use rekindle_types::friend::FriendState;

// ── Session ─────────────────────────────────────────────────────────────

/// Root session state for the local user.
#[derive(Debug, Clone)]
pub struct Session {
    /// The local user's identity.
    pub identity: SessionIdentity,

    /// Communities the user has joined, keyed by governance DHT key.
    pub communities: HashMap<String, CommunityMembership>,

    /// Our friends by public key, every friendship state (plan E2.1): loaded
    /// from `friends` at unlock and written after each repository write.
    pub friends: HashMap<String, FriendState>,

    /// Our friend-presence key (plan E2.5), the newest epoch: profile
    /// subkeys 2 and 4 are sealed under it. Minted at the first unlock.
    pub presence_key: Option<PresenceKey>,

    /// Each friend's current friend-presence key, which opens their STATUS
    /// and GAME_INFO (from `friends.presence_*`).
    pub friend_presence_keys: HashMap<String, PresenceKey>,
}

impl Session {
    /// Create a new session from a freshly created identity.
    pub fn new(identity: SessionIdentity) -> Self {
        Self {
            identity,
            communities: HashMap::new(),
            friends: HashMap::new(),
            presence_key: None,
            friend_presence_keys: HashMap::new(),
        }
    }

    /// Add a community membership. Overwrites if the anchor key already exists.
    pub fn join_community(&mut self, membership: CommunityMembership) {
        self.communities
            .insert(membership.anchor.clone(), membership);
    }

    /// Remove a community membership by anchor key.
    pub fn leave_community(&mut self, anchor: &str) {
        self.communities.remove(anchor);
    }

    /// Look up a community membership by anchor key.
    pub fn community(&self, anchor: &str) -> Option<&CommunityMembership> {
        self.communities.get(anchor)
    }

    /// Look up a community membership by community name (case-insensitive).
    ///
    /// Returns `None` if zero or more than one community matches. The CLI
    /// should use anchor key for unambiguous lookups; this is a
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
    pub anchor: String,

    /// Our pseudonym public key for this community (hex).
    pub pseudonym_key: String,

    /// Member registry DHT key for this community.
    pub registry_key: String,

    /// Our slot index in the member registry.
    pub slot_index: u32,

    /// Cached community name (from governance metadata).
    pub community_name: String,

    /// Slot seed bytes for deriving our SMPL writer keypair (the vault's
    /// `SlotSeed`).
    pub slot_seed: Option<[u8; 32]>,

    // ── v2.0 governance-runtime fields ──
    //
    // The daemon's `GovernanceRuntimeDeps` adapter reports these across
    // the Schwarzschild boundary as `governance_runtime::CommunityMembership`.
    /// Plate Gate segment hosting our slot (architecture §15).
    /// `None` = unknown/legacy; `0` = genesis segment. The global slot
    /// used for keypair derivation is `segment * SLOTS_PER_SEGMENT +
    /// slot_index`, so for segment 0 the two are equal.
    pub segment_index: Option<u32>,

    /// Highest community-level MEK generation we have seen.
    pub mek_generation: u64,
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
