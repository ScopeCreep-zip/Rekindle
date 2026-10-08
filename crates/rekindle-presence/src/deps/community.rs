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
    /// The scope of the session this work belongs to. A poll tick stops
    /// before its next Veilid call once it is closed (plan C4.L1).
    fn scope(&self) -> std::sync::Arc<rekindle_lifecycle::SessionScope>;

    // === Initial-sync handshake surface (21.k-REDO) ===

    /// Pseudonym key (hex) the local user holds in `community_id`,
    /// or empty string when missing.
    fn my_pseudonym_for_community(&self, community_id: &str) -> String;

    /// Our published private route blob (if attached), used by the
    /// gossip `PresenceUpdate` broadcast. This is the GENERAL route
    /// (Reliable + PreferOrdered) for chat / governance / gossip.
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

    /// The community's writer index: the slot of every member the
    /// presence scan placed in our segment, and our own. A channel
    /// catch-up reads only these slots, never the whole range (plan
    /// C7.12); slots are where members joined, not `0..member count`.
    async fn member_slots_for_community(&self, community_id: &str) -> Vec<u32>;

    /// Fire-and-forget gossip broadcast of a community envelope.
    /// Mirrors `services::community::send_to_mesh` semantics: the
    /// transport pipeline is spawned + errors logged inside.
    fn send_to_mesh(
        &self,
        community_id: &str,
        envelope: rekindle_codec::community::envelope::CommunityEnvelope,
    );

    /// Highest timestamp persisted locally for the given channel —
    /// the orchestrator uses this as the `since_timestamp` on
    /// `SyncRequest` envelopes.
    async fn last_channel_message_timestamp(&self, community_id: &str, channel_id: &str) -> i64;

    /// Record that a `SyncRequest` was just fired for the channel
    /// (so the retry sweep in `presence_poll_tick` can detect stale
    /// outstanding requests).
    fn mark_pending_sync(&self, community_id: &str, channel_id: &str, attempt: u32);

    /// Read the `member_slots` of an SMPL channel record that hold a
    /// value: each message with the subkey it was written to. The record
    /// key and subkey are the body's AAD position, so a reader that drops
    /// them cannot open the body.
    async fn read_channel_message_items(
        &self,
        record_key: &str,
        member_slots: &[u32],
    ) -> Result<
        Vec<(
            u32,
            rekindle_codec::community::channel_record::ChannelMessage,
        )>,
        PresenceError,
    >;

    /// Persist a batch of newly-fetched channel messages, read from
    /// `record_key`, into the local message log (skips rows already
    /// stored by message_id). The host opens each body at its position.
    fn persist_channel_catchup(
        &self,
        community_id: &str,
        channel_id: &str,
        record_key: &str,
        messages: Vec<(
            u32,
            rekindle_codec::community::channel_record::ChannelMessage,
        )>,
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

    /// The local user's current session for this community — typed
    /// status (mapped from identity `UserStatus`), focused channel
    /// (`my_session_location`), and last-active. Built under the
    /// communities lock. The single source of truth the write path
    /// derives the loose `status` string from.
    fn self_session(&self, community_id: &str) -> rekindle_types::presence::MemberSession;

    /// The local user's per-community presence sharing policy
    /// (default-deny). Local-only — never published. Applied to the
    /// session before signing + publishing.
    fn presence_policy(
        &self,
        community_id: &str,
    ) -> rekindle_types::presence::PresenceSharingPolicy;

    /// Encrypt the identity-revealing `SessionExtras` (location +
    /// activity) under the current community MEK so only current
    /// members can read them. `None` when MEK is missing OR there is
    /// nothing to share (caller omits the field).
    fn encrypt_session_extras_with_current_mek(
        &self,
        community_id: &str,
        extras: &rekindle_types::presence::SessionExtras,
    ) -> Option<rekindle_types::presence::EncryptedSessionExtras>;

    /// Decrypt a peer's MEK-encrypted `SessionExtras` during the
    /// roster read. `None` when we lack the matching MEK generation
    /// (the roster gracefully shows no location for that peer).
    fn decrypt_session_extras(
        &self,
        community_id: &str,
        encrypted: &rekindle_types::presence::EncryptedSessionExtras,
    ) -> Option<rekindle_types::presence::SessionExtras>;

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
    ) -> Result<RowWrite, PresenceError>;

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
    fn extend_known_members(&self, community_id: &str, candidates: Vec<String>) -> Vec<String>;

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

    // === presence_poll_tick surface (21.i-REDO) ===

    /// Ensure the community holds its member-registry record's lease in
    /// the record pool (plan C7.5), writable when a writer keypair is
    /// known, and return the record key. When the lease is already held
    /// this does no Veilid call; otherwise it borrows the record and hands
    /// the lease to the community. `Err` when the community isn't joined
    /// or the record cannot be opened.
    async fn ensure_registry_open(&self, community_id: &str) -> Result<Option<String>, String>;

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
    fn read_existing_member_roles(&self, community_id: &str) -> HashMap<String, Vec<u32>>;

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
        online_members: &mut HashMap<String, OnlineMember>,
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
        online_members: &HashMap<String, OnlineMember>,
        my_pseudonym: &str,
    ) -> Vec<String>;

    /// Snapshot the prior gossip overlay's mutation-relevant
    /// fields (lamport counter + needs_initial_sync flag + the
    /// drained pending-mesh queue). The crate orchestrator hands
    /// this to
    /// [`crate::community::overlay_rebuild::compute_rebuild_plan`]
    /// to compute the new overlay state.
    fn read_gossip_snapshot(&self, community_id: &str) -> crate::community::GossipOverlaySnapshot;

    /// Atomically write the rebuilt overlay back into the
    /// community state. The orchestrator passes the plan returned
    /// from `compute_rebuild_plan` after the write lock releases —
    /// the adapter does the actual `peers` / `online_members` /
    /// `needs_initial_sync` /
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
        envelope: rekindle_codec::community::envelope::SignedEnvelope,
    );

    /// Emit a `MemberPresenceChanged{status:"offline"}` community
    /// event for a peer that's just gone offline.
    fn emit_member_presence_offline(&self, community_id: &str, pseudonym_key: &str);

    /// Voice channel the LOCAL user is currently connected to in this
    /// community, if any. Rides the MEK-encrypted `SessionExtras` on
    /// our presence row (MatrixRTC `m.rtc.member` pattern) so the
    /// channel roster has a durable, heartbeat-renewed backstop.
    fn active_voice_channel(&self, community_id: &str) -> Option<String>;

    /// Hand the scan's presence-derived voice membership view to the
    /// voice layer for roster reconciliation (add lost-VoiceJoin
    /// members, expire ghosts). Fire-and-forget — the add/remove
    /// DECISIONS live in `rekindle-voice`; the adapter only bridges.
    fn reconcile_voice_roster(&self, community_id: &str, rows: Vec<VoicePresenceRow>);

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
pub use rekindle_types::presence::SegmentDescriptor;

/// How a presence-row write went (plan C7.16).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowWrite {
    /// Stored at consensus, or already the network's copy.
    Stored,
    /// The network held another value at our sequence number or above:
    /// Veilid adopted it and stored it locally (`set_value.rs:620-645`).
    Superseded { seq: Option<u32>, data: Vec<u8> },
}

/// Presence-derived voice membership view of one community member,
/// handed from the registry scan to the voice roster reconcile
/// (architecture three-path: the SMPL presence row is the DURABLE
/// roster; gossip VoiceJoin/VoiceLeave is the fast path).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoicePresenceRow {
    pub pseudonym_hex: String,
    pub display_name: Option<String>,
    /// The cleartext voice channel claim on the row. The row carries no
    /// media route (plan C7.15): the reconcile introduces us through the
    /// call's signaling instead.
    pub voice_channel_id: Option<String>,
    /// Row passed the scan's liveness gate (fresh heartbeat +
    /// non-offline). Stale rows still flow through so the reconcile
    /// can expire ghosts.
    pub fresh: bool,
}

/// In-memory snapshot of one online community member used by the
/// gossip overlay rebuild. Mirrors src-tauri's `OnlineMember`
/// shape without forcing the crate to depend on the AppState type.
///
/// `location` / `last_active` are the decoded session signals used by
/// the roster — `location` is filled by the orchestrator after the
/// scan (decrypting the MEK-bounded `SessionExtras`), so it defaults to
/// `None` on the bare classifier output.
// The online-member row is `rekindle_types::presence::OnlineMember`.
// This crate declared an identical five-field copy under a second name,
// which cost three hand-written identity converters in the src-tauri
// adapter alone (`online_member_from_state`,
// `state_online_from_snapshot`) — field-for-field copies that existed
// only because the two names were two types.
pub use rekindle_types::presence::OnlineMember;

/// Per-community profile fields the presence write path needs.
#[derive(Debug, Clone, Default)]
pub struct SelfPresenceSnapshot {
    pub bio: Option<String>,
    pub pronouns: Option<String>,
    pub theme_color: Option<u32>,
    pub badges: Vec<String>,
    pub avatar_ref: Option<String>,
    pub banner_ref: Option<String>,
}

/// The `community_members` row built from one discovered presence entry;
/// the table's repository owns the shape.
pub use rekindle_db::repo::members::DiscoveredMemberRow;
