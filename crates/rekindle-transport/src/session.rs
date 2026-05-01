//! Application-level session state.
//!
//! [`Session`] holds the local identity, community memberships, and DM
//! log key. It is owned by the application layer (CLI/TUI), not by the
//! transport node. The transport provides operations; the session holds
//! the context those operations need.
//!
//! Session state is persisted to `${XDG_STATE_HOME}/rekindle/session.json`
//! via atomic writes (tmp + fsync + rename). On startup the CLI loads it;
//! on every mutation the CLI writes it back. This gives crash-safe
//! persistence without a database.

use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{TransportError, Result};

// ── Session ─────────────────────────────────────────────────────────────

/// Root session state for the local user.
///
/// Serialized as JSON for human-inspectable persistence. Secret material
/// (keypair bytes) is excluded from serialization via `#[serde(skip)]` —
/// those are stored in the OS keyring, not on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// The local user's identity.
    pub identity: SessionIdentity,

    /// Communities the user has joined, keyed by governance DHT key.
    #[serde(default)]
    pub communities: HashMap<String, CommunityMembership>,

    /// DM conversation log DHT key, if created.
    #[serde(default)]
    pub dm_log_key: Option<String>,

    /// Pending inbound friend requests awaiting user action.
    /// Populated by the InboundHandler when a `DmPayload::FriendRequest`
    /// arrives. Drained when the user accepts or rejects.
    #[serde(default)]
    pub pending_friend_requests: Vec<PendingFriendRequest>,

    /// Schema version for forward compatibility.
    #[serde(default = "default_session_version")]
    pub version: u32,
}

fn default_session_version() -> u32 {
    1
}

impl Session {
    /// Create a new session from a freshly created identity.
    pub fn new(identity: SessionIdentity) -> Self {
        Self {
            identity,
            communities: HashMap::new(),
            dm_log_key: None,
            pending_friend_requests: Vec::new(),
            version: default_session_version(),
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

    /// Persist session to a JSON file via atomic write.
    ///
    /// Uses tmp + fsync + rename to prevent corruption on crash.
    pub fn save(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(self).map_err(|e| {
            TransportError::SerializationFailed {
                reason: format!("session: {e}"),
            }
        })?;
        atomic_write(path, json.as_bytes()).map_err(|e| {
            TransportError::Internal(format!("session write to {}: {e}", path.display()))
        })
    }

    /// Load session from a JSON file.
    ///
    /// Returns `None` if the file does not exist. Returns `Err` if the
    /// file exists but cannot be parsed.
    pub fn load(path: &Path) -> Result<Option<Self>> {
        match std::fs::read(path) {
            Ok(bytes) => {
                let session: Self = serde_json::from_slice(&bytes).map_err(|e| {
                    TransportError::DeserializationFailed {
                        type_id: 0,
                        reason: format!("session file {}: {e}", path.display()),
                    }
                })?;
                Ok(Some(session))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(TransportError::Internal(format!(
                "session read from {}: {e}",
                path.display()
            ))),
        }
    }
}

// ── Identity ────────────────────────────────────────────────────────────

/// The local user's cryptographic and network identity.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

    /// Profile DHT record keypair bytes (for re-opening writable).
    /// Excluded from JSON — stored in OS keyring.
    #[serde(skip)]
    pub profile_keypair_bytes: Option<Vec<u8>>,

    /// Friend list DHT record keypair bytes.
    /// Excluded from JSON — stored in OS keyring.
    #[serde(skip)]
    pub friend_list_keypair_bytes: Option<Vec<u8>>,
}

// ── Community membership ────────────────────────────────────────────────

/// Per-community membership state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunityMembership {
    /// Community governance DHT key.
    pub governance_key: String,

    /// Our pseudonym public key for this community (hex).
    pub pseudonym_key: String,

    /// Our display name within this community.
    pub display_name: String,

    /// Our role IDs in this community.
    #[serde(default)]
    pub role_ids: Vec<u32>,

    /// Member registry DHT key for this community.
    pub registry_key: String,

    /// Our slot index in the member registry.
    pub slot_index: u32,

    /// Cached community name (from governance metadata).
    pub community_name: String,

    /// Slot seed bytes for deriving our SMPL writer keypair.
    /// Excluded from JSON — stored in OS keyring.
    #[serde(skip)]
    pub slot_seed: Option<[u8; 32]>,
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

// ── Atomic file write ───────────────────────────────────────────────────

/// Write data to a file atomically: write to `.tmp`, fsync, rename.
///
/// This ensures the file is never in a partially-written state, even
/// if the process crashes or power is lost during the write.
fn atomic_write(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");

    // Ensure parent directory exists
    if let Some(parent) = tmp.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut file = std::fs::File::create(&tmp)?;
    file.write_all(data)?;
    file.sync_all()?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_identity() -> SessionIdentity {
        SessionIdentity {
            public_key_hex: "abcd1234".repeat(8),
            display_name: "alice".into(),
            profile_dht_key: "VLD0:profile:key".into(),
            mailbox_dht_key: "VLD0:mailbox:key".into(),
            friend_list_dht_key: "VLD0:friends:key".into(),
            profile_keypair_bytes: None,
            friend_list_keypair_bytes: None,
        }
    }

    fn test_membership() -> CommunityMembership {
        CommunityMembership {
            governance_key: "VLD0:gov:key".into(),
            pseudonym_key: "pseudonym_hex".into(),
            display_name: "alice_in_devteam".into(),
            role_ids: vec![1, 2],
            registry_key: "VLD0:reg:key".into(),
            slot_index: 3,
            community_name: "dev-team".into(),
            slot_seed: None,
        }
    }

    #[test]
    fn session_round_trip_json() {
        let mut session = Session::new(test_identity());
        session.join_community(test_membership());

        let json = serde_json::to_string_pretty(&session).unwrap();
        let restored: Session = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.identity.public_key_hex, session.identity.public_key_hex);
        assert_eq!(restored.identity.display_name, "alice");
        assert_eq!(restored.communities.len(), 1);
        assert!(restored.communities.contains_key("VLD0:gov:key"));
    }

    #[test]
    fn session_save_and_load() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("session.json");

        let mut session = Session::new(test_identity());
        session.join_community(test_membership());
        session.save(&path).unwrap();

        let loaded = Session::load(&path).unwrap().unwrap();
        assert_eq!(loaded.identity.display_name, "alice");
        assert_eq!(loaded.communities.len(), 1);
    }

    #[test]
    fn session_load_nonexistent_returns_none() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("does_not_exist.json");
        let result = Session::load(&path).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn session_load_corrupt_returns_err() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("corrupt.json");
        std::fs::write(&path, b"not valid json {{{").unwrap();
        assert!(Session::load(&path).is_err());
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

    #[test]
    fn skip_fields_not_serialized() {
        let mut identity = test_identity();
        identity.profile_keypair_bytes = Some(vec![1, 2, 3]);
        let session = Session::new(identity);

        let json = serde_json::to_string(&session).unwrap();
        // Skip fields should not appear in JSON
        assert!(!json.contains("profile_keypair_bytes"));
        assert!(!json.contains("friend_list_keypair_bytes"));
    }
}
