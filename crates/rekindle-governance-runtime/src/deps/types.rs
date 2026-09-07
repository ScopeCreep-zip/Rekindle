//! DTOs exchanged across the `GovernanceRuntimeDeps` boundary.
//!
//! Split from the trait itself to keep both files under the 600-line
//! ceiling. These are the Schwarzschild boundary's vocabulary: every
//! Veilid value crosses as an opaque `String` / `Vec<u8>`, and live
//! host structures cross as snapshots, so the crate never needs the
//! host's state types.

use std::collections::HashMap;

use rekindle_governance::state::GovernanceState;

/// User-status flavor without depending on src-tauri's `UserStatus` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserStatusKind {
    Online,
    Away,
    Busy,
    Offline,
    Invisible,
}

impl UserStatusKind {
    /// String form used in `MemberPresence.status` per architecture §13.4.
    #[must_use]
    pub fn as_wire_str(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Away => "away",
            Self::Busy => "busy",
            Self::Offline | Self::Invisible => "offline",
        }
    }
}

/// Snapshot of the fields of `CommunityState` that lifecycle ops read.
///
/// Batching into a single DTO mirrors how the src-tauri code already
/// accesses state — one locked-read, then multi-field destructure. The
/// adapter clones the parking_lot RwLockReadGuard contents into this
/// DTO so the lock is dropped before any `.await`.
#[derive(Debug, Clone)]
pub struct CommunityMembership {
    pub governance_key: Option<String>,
    pub member_registry_key: Option<String>,
    pub my_pseudonym_hex: Option<String>,
    pub my_subkey_index: Option<u32>,
    pub my_segment_index: Option<u32>,
    pub slot_keypair: Option<String>,
    pub slot_seed_hex: Option<String>,
    pub dht_owner_keypair: Option<String>,
    pub lamport_counter: u64,
    pub channel_log_keys: HashMap<String, String>,
    pub channel_ids: Vec<String>,
    pub mek_generation: u64,
}

/// Online member entry extracted from the gossip overlay's
/// `online_members` map. Adapter snapshots into Vec form so the crate
/// doesn't need access to the live `GossipOverlay` struct.
#[derive(Debug, Clone)]
pub struct OnlineMemberSnapshot {
    pub pseudonym_hex: String,
    pub status: String,
    pub route_blob: Vec<u8>,
    pub last_seen: u64,
}

/// MEK material exchanged across the crate boundary.
///
/// The wire bytes form lets the crate decode/re-encrypt without
/// importing `rekindle_crypto::group::media_key::MediaEncryptionKey` at
/// every callsite (it's used in many places but the trait stays opaque).
#[derive(Debug, Clone)]
pub struct MekSnapshot {
    pub generation: u64,
    /// 32-byte raw key material (the result of `MediaEncryptionKey::as_bytes`).
    pub key_bytes: [u8; 32],
}

/// Per-channel MEK pair returned from `channel_meks_all` for the
/// bootstrap response build.
#[derive(Debug, Clone)]
pub struct ChannelMekSnapshot {
    pub channel_id: String,
    pub mek: MekSnapshot,
}

/// Outcome of creating a new SMPL DHT record. Owner keypair string is
/// `None` if the underlying record was created with `o_cnt: 0` (the
/// universal community SMPL schema — Schwarzschild principle, §3).
#[derive(Debug, Clone)]
pub struct DhtRecordInfo {
    pub record_key: String,
    pub owner_keypair: Option<String>,
}

/// One row from the `messages` table used to build a bootstrap bundle.
#[derive(Debug, Clone)]
pub struct RecentMessageRow {
    pub message_id: String,
    pub sender_pseudonym: String,
    pub body: String,
    pub timestamp: i64,
    /// Original generation the message was stored under (architecture §5.2).
    pub mek_generation: u64,
}

/// Row returned from `read_member_index_for_registry`. The pseudonym
/// key is hex-encoded so the trait stays free of `rekindle-types::id`
/// constructors at callsites.
#[derive(Debug, Clone)]
pub struct MemberIndexRow {
    pub pseudonym_key_hex: String,
    pub subkey_index: u32,
    pub role_ids: Vec<u32>,
}

/// Snapshot of the per-community DHT-records info the
/// `open_community_dht_records` orchestrator needs. The adapter
/// builds this from a single `state.communities.read()` so the lock
/// is dropped before any `.await`.
#[derive(Debug, Clone)]
pub struct CommunityDhtOpenSetup {
    pub id: String,
    pub governance_key: String,
    pub registry_key: Option<String>,
    /// Preferred registry writer keypair string (registry_owner if
    /// present, falling back to the slot keypair). `None` opens the
    /// record read-only.
    pub registry_writer: Option<String>,
}

/// Slot row discovered from a presence/registry scan during join.
#[derive(Debug, Clone)]
pub struct DiscoveredMember {
    pub segment_index: u32,
    pub slot_index: u32,
    pub presence: rekindle_types::presence::MemberPresence,
    pub role_ids: Vec<u32>,
}

/// Fields needed to insert a freshly-created community (origin flow)
/// into the src-tauri `AppState.communities` map.
///
/// Constructed inside `origin::create_community` once all genesis writes
/// succeed. The adapter copies the contents into a fresh `CommunityState`.
#[derive(Debug, Clone)]
pub struct CommunityInsert {
    pub id: String,
    pub name: String,
    pub channel_id_hex: String,
    pub channel_record_key: String,
    pub governance_key: String,
    pub registry_key: String,
    pub registry_owner_keypair: Option<String>,
    pub dht_owner_keypair: Option<String>,
    pub slot_seed_hex: String,
    pub slot_keypair: String,
    pub my_pseudonym_hex: String,
    pub mek: MekSnapshot,
    pub governance_state: GovernanceState,
    pub lamport_counter: u64,
    pub creator_role_ids: Vec<u32>,
}
