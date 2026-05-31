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

use crate::error::{Result, TransportError};

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

    /// Legacy single DM log key (deprecated — use per-peer dm_log_keys).
    #[serde(default)]
    pub dm_log_key: Option<String>,

    /// Per-peer DM DhtLog spine keys. Maps peer_public_key → DhtLog spine key.
    /// Created during friend accept. Both peers read/write their shared log.
    #[serde(default)]
    pub dm_log_keys: HashMap<String, String>,

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
            dm_log_keys: HashMap::new(),
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
        // Append BLAKE3 MAC for integrity verification on load
        let mac = blake3::keyed_hash(session_mac_key(), json.as_bytes());
        let wrapper = format!("{json}\n---MAC---\n{}", hex::encode(mac.as_bytes()));
        atomic_write(path, wrapper.as_bytes()).map_err(|e| {
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
                let content = String::from_utf8_lossy(&bytes);
                // Split MAC from session JSON
                let (json_str, verified) = if let Some(idx) = content.rfind("\n---MAC---\n") {
                    let json_part = &content[..idx];
                    let mac_hex = content[idx + 11..].trim();
                    let expected = blake3::keyed_hash(session_mac_key(), json_part.as_bytes());
                    let stored = hex::decode(mac_hex).unwrap_or_default();
                    if stored.len() == 32 && stored == expected.as_bytes() {
                        (json_part.to_string(), true)
                    } else {
                        return Err(TransportError::Internal(
                            "session.json integrity check FAILED — file may be tampered".into(),
                        ));
                    }
                } else {
                    // No MAC separator — treat entire content as JSON (should not happen in normal flow)
                    tracing::warn!("session.json has no integrity MAC — will add on next save");
                    (content.into_owned(), false)
                };
                let _ = verified;
                let session: Self = serde_json::from_str(&json_str).map_err(|e| {
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

    /// Friend inbox DHT record key (DFLT(32), published keypair).
    /// Other users write friend requests here. Our daemon polls it.
    #[serde(default)]
    pub friend_inbox_key: String,

    /// Hex-encoded keypair for the friend inbox. Published in our
    /// profile so anyone can open the inbox for writing.
    #[serde(default)]
    pub friend_inbox_keypair_hex: String,

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

    /// Per-channel message record keys owned by this member.
    /// Maps channel_id → DHT record key (DFLT(1), member-owned).
    /// Each member creates and owns their own record per channel.
    /// Persisted in session.json so records survive restart.
    #[serde(default)]
    pub channel_record_keys: HashMap<String, String>,

    /// Community mailbox DHT key — the community's RPC endpoint.
    /// Used to send join requests and governance operations.
    #[serde(default)]
    pub community_mailbox_key: String,

    /// Join inbox DHT key (operators only). Used to match ValueChange events
    /// and trigger inbox processing for auto-approval of join requests.
    #[serde(default)]
    pub join_inbox_key: String,

    /// Whether this member is an operator (holds the governance keypair).
    /// Operators can execute governance writes on behalf of the community.
    #[serde(default)]
    pub is_operator: bool,

    /// OS keyring label for the governance keypair, if this member is an operator.
    /// Format: "community-governance-{short_key}"
    #[serde(skip)]
    pub governance_keypair_label: Option<String>,
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

// ── Session MAC key ─────────────────────────────────────────────────────

/// Application-scoped MAC key for session integrity.
///
/// TODO: This is a static key derived from a hardcoded salt. It protects
/// against accidental corruption and casual tampering, but NOT against a
/// sophisticated attacker who reads the source. For signing-key-based MAC,
/// we'd need to verify after unlock — which changes the startup flow.
fn session_mac_key() -> &'static [u8; 32] {
    static KEY: std::sync::OnceLock<[u8; 32]> = std::sync::OnceLock::new();
    KEY.get_or_init(|| *blake3::hash(b"rekindle-session-integrity-v1").as_bytes())
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

    // Set restrictive permissions before rename — session contains
    // DHT keys and community membership data.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }

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
            friend_inbox_key: "VLD0:friend-inbox:key".into(),
            friend_inbox_keypair_hex: String::new(),
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
            channel_record_keys: HashMap::new(),
            community_mailbox_key: "VLD0:mailbox:community".into(),
            join_inbox_key: String::new(),
            is_operator: false,
            governance_keypair_label: None,
        }
    }

    #[test]
    fn session_round_trip_json() {
        let mut session = Session::new(test_identity());
        session.join_community(test_membership());

        let json = serde_json::to_string_pretty(&session).unwrap();
        let restored: Session = serde_json::from_str(&json).unwrap();

        assert_eq!(
            restored.identity.public_key_hex,
            session.identity.public_key_hex
        );
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
