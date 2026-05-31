//! Deps trait surface for Phase 18 community lifecycle ops.
//!
//! One composite trait (`GovernanceRuntimeDeps`) abstracts every
//! src-tauri/Veilid/SQLite/Stronghold capability the crate's pure
//! orchestration logic needs. The adapter at
//! `src-tauri/src/services/governance_adapter.rs` implements it.
//!
//! Schwarzschild boundary: all Veilid types (`RecordKey`, `KeyPair`,
//! `RoutingContext`) are exchanged as opaque `String` / `Vec<u8>` here.
//! The crate never imports `veilid-core`. Likewise the SQLite-backed
//! recent-messages query is a single trait method returning a DTO
//! (`RecentMessageRow`).

use std::collections::HashMap;

use async_trait::async_trait;
use rekindle_governance::state::GovernanceState;
use rekindle_protocol::dht::community::envelope::CommunityEnvelope;
use rekindle_types::governance::GovernanceEntry;

use crate::error::GovernanceRuntimeError;
use crate::event::GovernanceRuntimeEvent;

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

/// Composite deps trait — one impl on the src-tauri adapter, every
/// Phase 18 fn is parameterized over `D: GovernanceRuntimeDeps`.
///
/// Follows the Phase 17 pattern (single composite trait with sync state
/// reads + async DHT/I/O methods). Methods are grouped by responsibility:
/// identity, community state read/write, DHT, MEK cache, gossip, events,
/// background lifecycle.
#[async_trait]
pub trait GovernanceRuntimeDeps: Send + Sync {
    // ---------- Identity ----------

    fn identity_secret(&self) -> Option<[u8; 32]>;
    fn identity_display_name(&self) -> String;
    fn identity_status(&self) -> UserStatusKind;
    fn our_route_blob(&self) -> Vec<u8>;

    // ---------- Community state (read) ----------

    fn community_membership(&self, community_id: &str) -> Option<CommunityMembership>;
    fn governance_state(&self, community_id: &str) -> Option<GovernanceState>;
    fn online_members(&self, community_id: &str) -> Vec<OnlineMemberSnapshot>;
    fn open_record_keys(&self, community_id: &str) -> Vec<String>;

    // ---------- Community state (mutation) ----------

    fn set_governance_state(&self, community_id: &str, state: GovernanceState);
    fn increment_lamport(&self, community_id: &str) -> u64;
    fn insert_community(&self, community: CommunityInsert);
    fn mark_open_channel_record(&self, community_id: &str, record_key: String);

    // ---------- DHT (Schwarzschild — bytes only) ----------

    async fn create_smpl_record(
        &self,
        member_pubkeys: &[[u8; 32]],
    ) -> Result<DhtRecordInfo, GovernanceRuntimeError>;

    /// Convert an Ed25519 `(public, secret)` byte pair into the string
    /// form that the adapter understands as `writer` for `set_dht_value`
    /// + persists in `CommunityState.slot_keypair`. Lives on the trait
    /// because the underlying construction needs `veilid_core::KeyPair`
    /// (forbidden in this crate per Invariant 2).
    fn format_writer_keypair(&self, ed_public: [u8; 32], ed_secret: [u8; 32]) -> String;

    async fn get_dht_value(
        &self,
        record_key: &str,
        subkey: u32,
        force_refresh: bool,
    ) -> Result<Option<Vec<u8>>, GovernanceRuntimeError>;

    /// Returns `Ok(None)` on success, `Ok(Some(stale_bytes))` if the
    /// network's view is newer (M9.5 write conflict per §Failure 4 in
    /// `governance.rs`).
    async fn set_dht_value(
        &self,
        record_key: &str,
        subkey: u32,
        value: Vec<u8>,
        writer: Option<String>,
    ) -> Result<Option<Vec<u8>>, GovernanceRuntimeError>;

    async fn inspect_dht_record_local_seqs(
        &self,
        record_key: &str,
    ) -> Result<Vec<u64>, GovernanceRuntimeError>;

    /// Network-authoritative inspect (Veilid `DHTReportScope::UpdateGet`)
    /// returning per-subkey seq numbers. Used during slot claim to confirm
    /// occupancy from the network (not just our cached view) before
    /// writing.
    async fn inspect_dht_record_update_get_seqs(
        &self,
        record_key: &str,
    ) -> Result<Vec<u64>, GovernanceRuntimeError>;

    async fn open_dht_record(
        &self,
        record_key: &str,
        writer: Option<String>,
    ) -> Result<(), GovernanceRuntimeError>;

    // ---------- MEK cache ----------

    fn community_mek(&self, community_id: &str) -> Option<MekSnapshot>;
    fn channel_mek(&self, community_id: &str, channel_id: &str) -> Option<MekSnapshot>;
    fn channel_meks_all(&self, community_id: &str) -> Vec<ChannelMekSnapshot>;
    fn insert_community_mek(&self, community_id: &str, mek: MekSnapshot);
    fn insert_channel_mek(&self, community_id: &str, channel_id: &str, mek: MekSnapshot);

    /// Load a historical channel MEK from Stronghold. Bootstrap rebuilds
    /// recent messages under their original generation (architecture §5.2
    /// line 1100).
    fn load_historical_channel_mek(
        &self,
        community_id: &str,
        channel_id: &str,
        generation: u64,
    ) -> Option<MekSnapshot>;

    // ---------- Bootstrap (SQL) ----------

    /// Pull the most recent `limit` messages for `(community, channel)`
    /// from SQLite, oldest sort order in the result Vec (the adapter
    /// reverses the `ORDER BY timestamp DESC` query).
    async fn recent_channel_messages(
        &self,
        community_id: &str,
        channel_id: &str,
        limit: i64,
    ) -> Vec<RecentMessageRow>;

    // ---------- Gossip ----------

    /// Broadcast a `CommunityEnvelope` via the mesh. The adapter
    /// delegates to `services::community::gossip::send_to_mesh` (Phase
    /// 20 destination — still in src-tauri at Phase 18 time). Sync
    /// because the existing src-tauri impl is sync.
    fn send_to_mesh(
        &self,
        community_id: &str,
        envelope: &CommunityEnvelope,
    ) -> Result<(), GovernanceRuntimeError>;

    // ---------- Permissions ----------

    /// Wrap `commands::community::require_permission` — returns
    /// `Err(PermissionDenied)` if the caller doesn't hold `perm_bits`.
    fn require_permission(
        &self,
        community_id: &str,
        perm_bits: u64,
    ) -> Result<(), GovernanceRuntimeError>;

    // ---------- Events ----------

    fn emit_event(&self, event: GovernanceRuntimeEvent);

    // ---------- Background lifecycle (origin/join) ----------

    fn spawn_inspect_loop(&self, community_id: &str);
    fn spawn_presence_poll(&self, community_id: &str);
    fn spawn_dht_keepalive(&self, community_id: &str);
    fn spawn_history_catchup(&self, community_id: &str);

    /// Subscribe to the community's DHT records (governance + registry +
    /// every channel). Async because Veilid's `watch_dht_values` is async.
    async fn watch_community_records(
        &self,
        community_id: &str,
    ) -> Result<(), GovernanceRuntimeError>;

    /// Open + warm the Lost Cargo file cache for a community.
    fn ensure_files_cache_open(&self, community_id: &str);

    /// Persist freshly-discovered registry members into SQLite +
    /// `MemberDiscovered` UI emit (the existing
    /// `presence::registry::persist_discovered_registry_members` helper).
    fn persist_discovered_registry_members(
        &self,
        community_id: &str,
        members: Vec<DiscoveredMember>,
    );

    // ---------- Join-flow specific ----------

    /// Send a `CommunityEnvelope`-encoded app_call to a peer (used by
    /// the join bootstrap-fetch and Plate Gate expand-request paths).
    async fn app_call_peer(
        &self,
        target_route_blob: &[u8],
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, GovernanceRuntimeError>;

    /// Re-merge the current governance state for a community after the
    /// joiner has applied bootstrap entries — gives `GovernanceState`
    /// back. Convenience over the rekindle-governance::merge call so the
    /// adapter can supply the canonical pseudonym + entry list.
    fn rebuild_governance_state(
        &self,
        entries: Vec<(rekindle_types::id::PseudonymKey, Vec<GovernanceEntry>)>,
    ) -> GovernanceState;

    // ---------- DHT-hydration deps (Phase 23.C chiral split) ----------

    /// Snapshot every joined community's `(community_id, governance_key)`.
    /// Communities without a `governance_key` (v1.0 legacy) are skipped.
    /// Used by `dht_hydration::rebuild_governance_from_dht` to enumerate
    /// records to merge.
    fn list_community_governance_targets(&self) -> Vec<(String, String)>;

    /// Snapshot every joined community's
    /// `(community_id, registry_key, my_pseudonym_hex_opt)`. Communities
    /// without a `member_registry_key` are skipped. Used by
    /// `dht_hydration::hydrate_community_state_from_dht` to enumerate
    /// records to read from the DHT.
    fn list_registries_with_my_pseudonym(&self) -> Vec<(String, String, Option<String>)>;

    /// Read the full member index from a community's SMPL registry
    /// record. Returns per-member `(pseudonym_key_hex, subkey_index,
    /// role_ids)` so the orchestrator can recover state for the
    /// logged-in user without touching `veilid-core` or protocol types
    /// directly.
    async fn read_member_index_for_registry(
        &self,
        registry_key: &str,
    ) -> Result<Vec<MemberIndexRow>, GovernanceRuntimeError>;

    /// Apply recovered member state for one community: writes
    /// `my_subkey_index` if missing, updates `my_role_ids` if the DHT
    /// view is richer, then persists both to SQLite so the next login
    /// can skip the recovery step. Matches the pre-Phase-23 semantics
    /// of the inline body.
    fn apply_recovered_member_state(
        &self,
        community_id: &str,
        subkey_index: u32,
        role_ids: &[u32],
    );

    /// If the community has a `slot_seed` + `my_subkey_index` but no
    /// `slot_keypair`, derive the keypair now via the existing
    /// `services::community::try_derive_slot_keypair`. No-op otherwise.
    fn try_derive_slot_keypair_if_ready(&self, community_id: &str);

    /// Belt-and-suspenders: list communities whose
    /// `registry_owner_keypair` is empty. Used by the orchestrator to
    /// trigger Stronghold recovery for races where login didn't load
    /// the keypair in time.
    fn list_missing_registry_keypairs(&self) -> Vec<String>;

    /// Look up the registry owner keypair from Stronghold and install
    /// it on the community's state. No-op when the keystore lookup
    /// returns `None`.
    fn recover_registry_keypair_from_keystore(&self, community_id: &str);

    /// Snapshot every joined community's open-DHT-records setup info.
    /// Used by `dht_hydration::open_community_dht_records` to drive
    /// the per-community `open_dht_record` calls without holding the
    /// `state.communities` read-guard across `.await`.
    fn list_communities_for_dht_open(&self) -> Vec<CommunityDhtOpenSetup>;

    /// Snapshot the channel DHTLog record keys for one community.
    /// Returned in unspecified order; the orchestrator opens each in turn.
    fn channel_log_keys_for_community(&self, community_id: &str) -> Vec<String>;

    /// Track every opened DHT record key on the live DHT manager so
    /// `shutdown_node` can close them in bulk (`state.dht_manager`
    /// `open_records` set).
    fn track_open_dht_records(&self, keys: &[String]);

    /// Persist the per-community `open_community_records` snapshot
    /// after a successful open pass — `governance_key` + `registry_key`
    /// + `registry_writer` + `channel_keys` + `records_open = true`.
    fn mark_community_records_open(
        &self,
        community_id: &str,
        governance_key: &str,
        registry_key: Option<&str>,
        registry_writer: Option<&str>,
        channel_keys: Vec<String>,
    );

    /// Subscribe to Veilid watch updates for the community's
    /// governance + registry + channel records. Best-effort; errors
    /// are logged inside the adapter.
    async fn watch_community_records_post_open(&self, community_id: &str);

    /// Apply the post-merge result for one community:
    /// 1. raise `lamport_counter` to `max(current, max_lamport)`,
    /// 2. install the new `GovernanceState` via `set_governance_state`,
    /// 3. persist the merged snapshot to SQLite so it survives restarts.
    ///
    /// Async because the SQLite persist is async. Returns when the
    /// in-memory + on-disk state are both updated.
    async fn apply_governance_rebuild_result(
        &self,
        community_id: &str,
        gov_state: GovernanceState,
        max_lamport: u64,
    );

    /// Fire-and-forget spawn of a text-MEK rotation task for a newly
    /// observed ban. Delegates to the existing
    /// `services::community::rotate_text_mek_for_departure`. The
    /// orchestrator iterates `new_bans` and calls this once per banned
    /// pseudonym so the rotation work is parallelised across bans.
    fn spawn_text_mek_rotation_for_ban(&self, community_id: &str, banned_pseudonym_hex: &str);

    // ---------- Role mutations (Phase 23.D.15) ----------

    /// Snapshot of an existing role's current definition. Used by
    /// `edit_role` to compute the merged-update RoleDefinition and by
    /// `resolve_self_assignable_pseudonym` to check `self_assignable`.
    fn role_current_definition(
        &self,
        community_id: &str,
        role_id: u32,
    ) -> Option<crate::roles::RoleSnapshotInsert>;

    /// Return `(existing_ids, next_position)` for `create_role` —
    /// used to allocate a unique random role id and assign the next
    /// position slot.
    fn role_table_summary(&self, community_id: &str) -> (Vec<u32>, i32);

    /// Apply a role-assignment delta: append `role_id` to
    /// `community_members.role_ids` for the given pseudonym; if
    /// `is_self`, also append to `community.my_role_ids`.
    async fn apply_role_assignment(
        &self,
        community_id: &str,
        pseudonym_key: &str,
        role_id: u32,
        is_self: bool,
    ) -> Result<(), GovernanceRuntimeError>;

    async fn apply_role_unassignment(
        &self,
        community_id: &str,
        pseudonym_key: &str,
        role_id: u32,
        is_self: bool,
    ) -> Result<(), GovernanceRuntimeError>;

    /// Insert a new role into AppState + DB. The `RoleSnapshotInsert`
    /// is fully populated; adapter mirrors into `community.roles` (sort
    /// by position) + `community_roles` SQLite row.
    async fn apply_role_create(
        &self,
        community_id: &str,
        snapshot: crate::roles::RoleSnapshotInsert,
    ) -> Result<(), GovernanceRuntimeError>;

    /// Apply a partial-update patch to an existing role. Adapter
    /// mutates AppState `community.roles[role_id]` + emits the matching
    /// `community_roles` UPDATE.
    async fn apply_role_edit(
        &self,
        community_id: &str,
        role_id: u32,
        patch: crate::roles::RoleSnapshotPatch,
    ) -> Result<(), GovernanceRuntimeError>;

    /// Remove `role_id` from AppState + DB + every member row that
    /// references it. Also drops the role from `my_role_ids` if
    /// present.
    async fn apply_role_delete(
        &self,
        community_id: &str,
        role_id: u32,
    ) -> Result<(), GovernanceRuntimeError>;
}
