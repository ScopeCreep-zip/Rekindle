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

    /// The host's clock port in seconds (plan C9.2): heartbeats, join
    /// times and the poll's freshness checks read it, never the wall clock.
    fn now_secs(&self) -> u64;

    /// Pseudonym key (hex) the local user holds in `community_id`,
    /// or empty string when missing.
    fn my_pseudonym_for_community(&self, community_id: &str) -> String;

    /// Our published private route blob (if attached), written on our
    /// presence row. This is the GENERAL route
    /// (Reliable + PreferOrdered) for chat / governance / gossip.
    fn our_route_blob(&self) -> Option<Vec<u8>>;

    /// Return once our general route is live: the record port's own
    /// `available_own(General)`, the wait the own-records stage takes, so
    /// the first presence row names a route peers can import (live
    /// finding 25). At once when no transport runs: the tick reports that.
    async fn own_route_available(&self);

    // === Registry write surface (21.j-REDO) ===

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
    /// shape; no presence row names the member (plan E3.4 S6: a name is
    /// the member's verified card's).
    fn emit_member_discovered(&self, community_id: &str, pseudonym_key: &str, subkey_index: u32);

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

    /// Apply the post-scan member-state update: drop bans from
    /// every in-memory collection, extend `known_members` with the
    /// freshly-scanned keys. Member roles are the merged governance's
    /// role assignments, which no scan writes. The crate orchestrator
    /// computes the inputs; this method does the state write under one
    /// lock.
    fn apply_member_state_update(
        &self,
        community_id: &str,
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
    Superseded { seq: u32, data: Vec<u8> },
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

/// In-memory snapshot of one online community member a fresh registry
/// row decodes to. Mirrors src-tauri's `OnlineMember`
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
