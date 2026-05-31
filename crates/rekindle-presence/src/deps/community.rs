//! Phase 21 REDO — `CommunityPresenceDeps` composite trait + DTOs.
//!
//! Bag of operations the community-presence orchestrators need.
//! Implemented alongside [`crate::deps::FriendPresenceDeps`] by the
//! same `PresenceAdapter` in src-tauri.

use std::collections::{HashMap, HashSet};

use async_trait::async_trait;

use crate::deps::PresenceError;

#[async_trait]
pub trait CommunityPresenceDeps: Send + Sync + 'static {
    // === Initial-sync handshake surface (21.k-REDO) ===

    /// Pseudonym key (hex) the local user holds in `community_id`,
    /// or empty string when missing.
    fn my_pseudonym_for_community(&self, community_id: &str) -> String;

    /// Our published private route blob (if attached), used by the
    /// gossip `PresenceUpdate` broadcast.
    fn our_route_blob(&self) -> Option<Vec<u8>>;

    /// String form of the local user's current presence status —
    /// "online" / "away" / "busy" / "offline" (Invisible folds to
    /// "offline"). Defaults to "online" when no identity is loaded.
    ///
    /// `community_id` is the wire-up point for per-community
    /// `MemberPresence.custom_status` (architecture spec
    /// `rekindle-communities-architecture.md` line 754 — custom
    /// status text like "Playing Halo" is published per-community).
    /// The current adapter impl returns global identity status; the
    /// parameter stays so a follow-up commit can resolve
    /// `community.my_custom_status` first and fall back to identity
    /// status without breaking the trait signature.
    fn current_presence_status_str(&self, community_id: &str) -> String;

    /// List of every joined channel in the community.
    fn channel_ids_for_community(&self, community_id: &str) -> Vec<String>;

    /// `(channel_id, smpl_record_key)` pairs for every channel that
    /// has an SMPL log record allocated.
    fn channel_log_keys_for_community(&self, community_id: &str) -> Vec<(String, String)>;

    /// Total known-member count (capped at u32). The orchestrator
    /// uses this as the upper bound when scanning SMPL channel
    /// records (one subkey per member).
    fn member_count_for_community(&self, community_id: &str) -> u32;

    /// Fire-and-forget gossip broadcast of a community envelope.
    /// Mirrors `services::community::send_to_mesh` semantics: the
    /// transport pipeline is spawned + errors logged inside.
    fn send_to_mesh(
        &self,
        community_id: &str,
        envelope: rekindle_protocol::dht::community::envelope::CommunityEnvelope,
    );

    /// Highest timestamp persisted locally for the given channel —
    /// the orchestrator uses this as the `since_timestamp` on
    /// `SyncRequest` envelopes.
    async fn last_channel_message_timestamp(
        &self,
        community_id: &str,
        channel_id: &str,
    ) -> i64;

    /// Record that a `SyncRequest` was just fired for the channel
    /// (so the retry sweep in `presence_poll_tick` can detect stale
    /// outstanding requests).
    fn mark_pending_sync(&self, community_id: &str, channel_id: &str, attempt: u32);

    /// Read every populated subkey from the SMPL channel log record.
    /// Returns the (sequence, sender, ciphertext, timestamp, …)
    /// tuples that haven't been broadcast yet via mesh, so the
    /// orchestrator can persist them into the local message log.
    async fn read_all_channel_messages(
        &self,
        record_key: &str,
        member_count: u32,
    ) -> Result<Vec<rekindle_protocol::dht::community::channel_record::ChannelMessage>, PresenceError>;

    /// Persist a batch of newly-fetched channel messages into the
    /// local message log (skips rows already stored by message_id).
    fn persist_channel_catchup(
        &self,
        community_id: &str,
        channel_id: &str,
        messages: Vec<rekindle_protocol::dht::community::channel_record::ChannelMessage>,
    );

    /// Flip `gossip.needs_initial_sync` to false after the initial
    /// sync round completes.
    fn mark_initial_sync_done(&self, community_id: &str);

    // === Registry write surface (21.j-REDO) ===

    /// Local user's display name (used as the default for the
    /// presence row's `display_name` field).
    fn identity_display_name(&self) -> String;

    /// Snapshot of every per-community profile field the presence
    /// row carries — read once under the communities lock so the
    /// write path doesn't have to juggle six clones inline.
    fn self_presence_snapshot(&self, community_id: &str) -> SelfPresenceSnapshot;

    /// W11.2 — encrypt the local history ranges with the current
    /// community MEK. Returns `None` when MEK is missing (the caller
    /// gracefully omits the field).
    fn encrypt_history_ranges_with_current_mek(
        &self,
        community_id: &str,
        ranges: &[rekindle_types::presence::HistoryRange],
    ) -> Option<rekindle_types::presence::EncryptedHistoryRanges>;

    /// Compute history ranges from the local message log for the
    /// Shared Locker pattern (architecture §14.3). DB-backed.
    async fn compute_history_ranges(
        &self,
        community_id: &str,
    ) -> Vec<rekindle_types::presence::HistoryRange>;

    /// Architecture §26 W26 — sign the presence row's canonical
    /// bytes with the community pseudonym signing key. Returns
    /// `None` when credentials are unavailable (the write is
    /// skipped).
    fn sign_presence_row(&self, community_id: &str, signing_bytes: &[u8]) -> Option<Vec<u8>>;

    /// Write a fully-signed presence row to the registry record at
    /// our subkey index. The writer keypair (slot keypair) is
    /// supplied as a string so the trait stays free of `veilid_core::KeyPair`.
    async fn write_presence_to_registry_subkey(
        &self,
        registry_key: &str,
        subkey_index: u32,
        presence_json: Vec<u8>,
        writer_keypair_str: &str,
    ) -> Result<(), PresenceError>;

    /// Persist a batch of discovered presence rows into
    /// `community_members` (one upsert per row, plus deletes for
    /// banned members). `joined_at` is the unix-seconds stamp used
    /// for first-seen rows.
    fn persist_discovered_member_rows(
        &self,
        community_id: &str,
        rows: Vec<DiscoveredMemberRow>,
        banned_pseudonyms: Vec<String>,
        joined_at: i64,
    );

    /// Update `community.known_members` with the freshly-scanned
    /// keys. Returns the subset that wasn't previously known (so
    /// the caller can fire `MemberDiscovered` events).
    fn extend_known_members(
        &self,
        community_id: &str,
        candidates: Vec<String>,
    ) -> Vec<String>;

    /// Fire a `MemberDiscovered` community event for a freshly-seen
    /// pseudonym. The adapter renders the matching src-tauri event
    /// shape.
    fn emit_member_discovered(
        &self,
        community_id: &str,
        pseudonym_key: &str,
        display_name: &str,
        subkey_index: u32,
    );

    // === Outer-loop hook (21.h-REDO) ===

    /// Run one presence-poll tick for the community. The crate's
    /// `start_presence_poll` cadence loop invokes this from its
    /// timer ticks; the adapter implements it by delegating to the
    /// crate's `presence_poll_tick` orchestrator.
    ///
    /// Returns `Err` with a human-readable reason when the tick
    /// fails (`"not attached"`, `"community not found"`); the outer
    /// loop logs and continues.
    async fn run_presence_poll_tick(&self, community_id: &str) -> Result<(), String>;

    /// Install the shutdown sender on the community so callers
    /// (`leave_community`, `cleanup`) can stop the loop. Pre-port
    /// this was set on `community.presence_poll_shutdown_tx`
    /// directly inside `start_presence_poll`; lifted here so the
    /// crate's outer loop can install it before the first tick
    /// fires.
    fn install_presence_poll_shutdown(
        &self,
        community_id: &str,
        shutdown_tx: tokio::sync::mpsc::Sender<()>,
    );

    // === presence_poll_tick surface (21.i-REDO) ===

    /// Ensure the member-registry DHT record is open for read +
    /// write (best effort) and return its key. Adapter mutates
    /// `community.open_community_records` as a side effect. `Err`
    /// when the community isn't joined or DHT isn't attached.
    async fn ensure_registry_open(
        &self,
        community_id: &str,
    ) -> Result<Option<String>, String>;

    /// Snapshot the local user's per-community presence credentials:
    /// pseudonym key (hex) + assigned subkey index + resolved slot
    /// keypair (lazy derivation when `slot_keypair` is missing but
    /// `slot_seed` + `subkey_index` are present) + segment index.
    /// Returns `None` when the community isn't joined.
    fn presence_credentials(&self, community_id: &str) -> Option<PresenceCredentials>;

    /// Governance-mandated ban list as hex-encoded pseudonym keys.
    fn governance_bans(&self, community_id: &str) -> HashSet<String>;

    /// Plate Gate segment descriptors (architecture §15.5) — one
    /// entry per allocated SMPL segment. Each descriptor carries
    /// the segment index + its own registry-record key.
    fn segment_descriptors(&self, community_id: &str) -> Vec<SegmentDescriptor>;

    /// Fetch every populated subkey from one segment's registry
    /// record as raw bytes. Pure transport: the adapter pumps the
    /// 0..`max_subkey` range through a semaphore-throttled
    /// `get_dht_value` (skipping `skip_subkey` if set) and returns
    /// `(subkey, raw_bytes)` tuples for every subkey that had a
    /// non-empty payload. The crate orchestrator then runs
    /// [`crate::community::scan_row::parse_and_classify_row`] per
    /// result for the W26 signature verify + ban filter + heartbeat
    /// classification — keeps that business logic out of the adapter.
    async fn scan_segment_raw(
        &self,
        registry_key: &str,
        max_subkey: u32,
        skip_subkey: Option<u32>,
    ) -> Vec<(u32, Vec<u8>)>;

    /// Snapshot the in-memory `community.member_roles` map for
    /// `community_id` (read-only). The crate orchestrator composes
    /// this with the governance assignments + local role-ids to
    /// produce the merged map via
    /// [`crate::community::role_merge::compute_merged_roles`].
    fn read_existing_member_roles(
        &self,
        community_id: &str,
    ) -> HashMap<String, Vec<u32>>;

    /// Snapshot the merged-governance `role_assignments` map
    /// (keyed by pseudonym) for `community_id`. Returns empty when
    /// the community has no governance state loaded yet.
    fn read_governance_role_assignments(
        &self,
        community_id: &str,
    ) -> HashMap<rekindle_types::id::PseudonymKey, HashSet<rekindle_types::id::RoleId>>;

    /// Local user's role-ids for `community_id` (the authoritative
    /// override that overrides governance-state lag).
    fn read_my_role_ids(&self, community_id: &str) -> Vec<u32>;

    /// Apply the post-scan member-state update: drop bans from
    /// every in-memory collection, extend `known_members` with the
    /// freshly-scanned keys, replace `member_roles` with the
    /// merged map. The crate orchestrator computes the inputs;
    /// this method does the AppState write under one lock.
    fn apply_member_state_update(
        &self,
        community_id: &str,
        merged_member_roles: HashMap<String, Vec<u32>>,
        known_member_keys: HashSet<String>,
        banned_members: &HashSet<String>,
    );

    /// Load the local user's known event IDs from the community
    /// events SQLite table — bounds the per-event RSVP aggregation
    /// so stale snapshots don't surface unloaded events.
    async fn load_known_event_ids(&self, community_id: &str) -> Vec<String>;

    /// Snapshot the local user's per-community `my_event_rsvps`
    /// map (event_id → status string).
    fn read_my_event_rsvps(&self, community_id: &str) -> HashMap<String, String>;

    /// Replace `community.event_rsvps_by_event` with the freshly
    /// aggregated map. The crate orchestrator computes the
    /// aggregation via
    /// [`crate::community::rsvp_aggregate::aggregate_event_rsvps`].
    fn write_event_rsvps_by_event(
        &self,
        community_id: &str,
        aggregated: HashMap<String, Vec<crate::community::EventRsvpEntry>>,
    );

    /// Snapshot the in-memory `community.member_profiles` map for
    /// the diff. The crate orchestrator hands this to
    /// [`crate::community::profile_diff::compute_profile_diff`]
    /// + applies the diff via [`apply_member_profile_updates`].
    fn read_member_profile_snapshot(
        &self,
        community_id: &str,
    ) -> HashMap<String, crate::community::MemberProfileSnapshot>;

    /// Apply the `updates` map into `community.member_profiles`
    /// (insert / overwrite each entry). When `emit_refreshed` is
    /// true the adapter also fires `CommunityEvent::MembersRefreshed`
    /// so the frontend re-fetches.
    fn apply_member_profile_updates(
        &self,
        community_id: &str,
        updates: HashMap<String, crate::community::MemberProfileSnapshot>,
        emit_refreshed: bool,
    );

    /// Re-inject still-fresh peers from the prior gossip overlay
    /// into `online_members` (architecture §3 — TTL-based eviction
    /// over a 180 s threshold so a peer briefly missing from one
    /// scan doesn't drop them from the overlay).
    fn extend_online_with_recent_gossip(
        &self,
        community_id: &str,
        online_members: &mut HashMap<String, OnlineMemberSnapshot>,
        my_pseudonym: &str,
        eviction_threshold_secs: u64,
    );

    /// Peers that WERE in the prior gossip overlay's online set but
    /// are absent from the freshly-scanned `online_members`. Used
    /// to emit one `MemberPresenceChanged{status:"offline"}` event
    /// per gone-offline peer.
    fn gossip_offline_diff(
        &self,
        community_id: &str,
        online_members: &HashMap<String, OnlineMemberSnapshot>,
        my_pseudonym: &str,
    ) -> Vec<String>;

    /// Snapshot the prior gossip overlay's mutation-relevant
    /// fields (lamport counter + needs_initial_sync flag + the
    /// drained pending-mesh queue). The crate orchestrator hands
    /// this to
    /// [`crate::community::overlay_rebuild::compute_rebuild_plan`]
    /// to compute the new overlay state.
    fn read_gossip_snapshot(
        &self,
        community_id: &str,
    ) -> crate::community::GossipOverlaySnapshot;

    /// Atomically write the rebuilt overlay back into the
    /// community state. The orchestrator passes the plan returned
    /// from `compute_rebuild_plan` after the write lock releases —
    /// the adapter does the actual `peers` / `online_members` /
    /// `lamport_counter` / `needs_initial_sync` /
    /// `pending_mesh_broadcasts` writes under one lock.
    fn apply_gossip_rebuild_plan(
        &self,
        community_id: &str,
        plan: crate::community::GossipOverlayPlan,
    );

    /// Resend a drained signed envelope through the gossip mesh.
    /// Adapter forwards to `services::community::send_to_mesh_raw`.
    fn send_to_mesh_raw(
        &self,
        community_id: &str,
        envelope: rekindle_protocol::dht::community::envelope::SignedEnvelope,
    );

    /// Emit a `MemberPresenceChanged{status:"offline"}` community
    /// event for a peer that's just gone offline.
    fn emit_member_presence_offline(&self, community_id: &str, pseudonym_key: &str);

    /// Pending-sync entries the orchestrator should retry (older
    /// than `stale_window_secs` with attempt count below
    /// `max_attempts`).
    fn stale_pending_syncs(
        &self,
        community_id: &str,
        now_secs: u64,
        stale_window_secs: u64,
        max_attempts: u32,
    ) -> Vec<(String, u32)>;

    /// Update the pending-sync timestamp + attempt count for a
    /// channel. Pre-port wrote `pending_syncs.insert(channel, (ts, attempt))`.
    fn update_pending_sync(
        &self,
        community_id: &str,
        channel_id: &str,
        now_secs: u64,
        attempt: u32,
    );

    /// Drop pending-sync entries that have hit their attempt cap.
    fn prune_pending_syncs(&self, community_id: &str, max_attempts: u32);

    /// Architecture §15 A5/P4.3 — admin-side trigger for Plate
    /// Gate segment expansion. Spawns a background task; returns
    /// immediately. No-op when caller lacks `MANAGE_COMMUNITY`
    /// permission or when the highest segment isn't full.
    fn maybe_auto_expand_segment(&self, community_id: &str);
}

// ---------- DTOs consumed by `CommunityPresenceDeps` ----------

/// Snapshot of the local user's per-community presence credentials.
#[derive(Debug, Clone)]
pub struct PresenceCredentials {
    pub my_pseudonym_hex: String,
    pub my_subkey_index: Option<u32>,
    pub slot_keypair_str: Option<String>,
    pub slot_seed_hex: Option<String>,
    pub my_segment_index: u32,
}

/// Plate Gate segment descriptor (architecture §15.5).
#[derive(Debug, Clone)]
pub struct SegmentDescriptor {
    pub segment_index: u32,
    pub registry_key: String,
}

/// In-memory snapshot of one online community member used by the
/// gossip overlay rebuild. Mirrors src-tauri's `OnlineMember`
/// shape (route_blob + status + last_seen) without forcing the
/// crate to depend on the AppState type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnlineMemberSnapshot {
    pub route_blob: Vec<u8>,
    pub status: String,
    pub last_seen: u64,
}

/// Per-community profile fields the presence write path needs.
#[derive(Debug, Clone, Default)]
pub struct SelfPresenceSnapshot {
    pub event_rsvps: Vec<rekindle_types::presence::EventRSVP>,
    pub bio: Option<String>,
    pub pronouns: Option<String>,
    pub theme_color: Option<u32>,
    pub badges: Vec<String>,
    pub avatar_ref: Option<String>,
    pub banner_ref: Option<String>,
}

/// Materialised SQLite row built from one discovered presence entry.
/// Mirrors the pre-port `MemberPersistRow` shape so the upsert path
/// in the adapter stays trivially transcribable.
#[derive(Debug, Clone)]
pub struct DiscoveredMemberRow {
    pub pseudonym_key: String,
    pub display_name: Option<String>,
    pub role_ids_json: String,
    pub subkey_index: i64,
    pub segment_index: i64,
    pub bio: Option<String>,
    pub pronouns: Option<String>,
    pub theme_color: Option<i64>,
    pub badges_json: String,
    pub avatar_ref: Option<String>,
    pub banner_ref: Option<String>,
}
