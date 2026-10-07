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

use async_trait::async_trait;
use rekindle_codec::community::envelope::CommunityEnvelope;
use rekindle_governance::state::GovernanceState;
use rekindle_records::lease::{CommunityLeases, LeaseId};
use rekindle_types::governance::GovernanceEntry;

use crate::error::GovernanceRuntimeError;
use crate::event::GovernanceRuntimeEvent;

mod types;

pub use types::*;

/// Composite deps trait — one impl on the src-tauri adapter, every
/// Phase 18 fn is parameterized over `D: GovernanceRuntimeDeps`.
///
/// Follows the Phase 17 pattern (single composite trait with sync state
/// reads + async DHT/I/O methods). Methods are grouped by responsibility:
/// identity, community state read/write, DHT, MEK cache, gossip, events,
/// background lifecycle.
// `automock` must precede `async_trait` — the generated mock has to see
// the un-desugared signatures. Test-only: no mock code ships.
//
// The allow covers mockall's *generated* body, which stores
// expectations in `std::sync::Mutex`.
//
// That ban is architecture rule B9, and it is a security rule, not a
// preference: `std::sync::Mutex` poisons on panic, so a panic reachable
// from attacker-supplied input turns into permanent unavailability of
// that lock — a DoS amplifier against a node whose whole job is
// processing untrusted peer data (threat model A2-A4). parking_lot does
// not poison, so the subsystem survives.
//
// The exemption is narrow and does not touch that property: this is
// `cfg(test)` only, so no mock code is ever linked into a shipped
// binary. What is being exempted is a test harness, not a code path an
// attacker can reach. The alternative — hand-writing a 66-method double
// — is exactly what left the slot-claim retry untested.
#[cfg_attr(
    test,
    allow(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        clippy::ref_option_ref,
        reason = "mockall's generated mock is test-only code we do not author; see B9 note above"
    )
)]
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait GovernanceRuntimeDeps: Send + Sync {
    // ---------- Session ----------

    /// The scope of the session this work belongs to. Multi-call
    /// orchestrators check it before each Veilid call and stop once it is
    /// closed (plan C4.L1).
    fn scope(&self) -> std::sync::Arc<rekindle_lifecycle::SessionScope>;

    // ---------- Identity ----------

    fn identity_secret(&self) -> Option<[u8; 32]>;
    fn identity_display_name(&self) -> String;
    fn identity_status(&self) -> UserStatusKind;
    fn our_route_blob(&self) -> Vec<u8>;

    // ---------- Community state (read) ----------

    fn community_membership(&self, community_id: &str) -> Option<CommunityMembership>;
    fn governance_state(&self, community_id: &str) -> Option<GovernanceState>;
    fn online_members(&self, community_id: &str) -> Vec<OnlineMemberSnapshot>;

    // ---------- Community state (mutation) ----------

    fn set_governance_state(&self, community_id: &str, state: GovernanceState);
    /// Next governance-clock value for a `GovernanceEntry` we write.
    fn next_governance_lamport(
        &self,
        community_id: &str,
    ) -> Result<u64, rekindle_types::lamport::LamportError>;
    fn insert_community(&self, community: CommunityInsert);

    // ---------- DHT (Schwarzschild — bytes only) ----------
    //
    // Records are borrowed from the host's record pool (plan C7): every
    // read, write and inspect names a lease, never a bare key. A record the
    // community keeps for the session is acquired once and handed to the
    // host by `community_records_ready`; anything else is acquired, used
    // and released. While a session lease is held, an acquire is a table
    // hit with no Veilid call.

    /// Borrow `record_key`, writable with `writer` (the string form of
    /// [`Self::format_writer_keypair`]) when given. The writer is sticky:
    /// a later read-only borrow never downgrades it.
    async fn acquire_record(
        &self,
        record_key: &str,
        writer: Option<String>,
    ) -> Result<LeaseId, GovernanceRuntimeError>;

    /// End a borrow; the record closes when it was the last.
    async fn release_record(&self, lease: LeaseId);

    /// Hand the community's session leases to the host. The host merges
    /// them into the community's held set (a lease on a record the set
    /// already holds is released, so repeat hand-overs do not accumulate),
    /// watches the records that were added, and starts its per-community
    /// loops (inspect, keepalive, presence poll) the first time, in its own
    /// scope. Called by origin, hydration, a new channel, a segment
    /// expansion and overflow pages.
    async fn community_records_ready(&self, community_id: &str, leases: CommunityLeases);

    async fn create_smpl_record(
        &self,
        member_pubkeys: &[[u8; 32]],
    ) -> Result<DhtRecordInfo, GovernanceRuntimeError>;

    /// Create a single-owner DFLT(1) record for one invite's encrypted
    /// `InviteSecrets` blob. `owner_keypair` is `Some` (DFLT owns subkey 0)
    /// so the caller can write the blob via `set_dht_value`.
    async fn create_dflt_record(&self) -> Result<DhtRecordInfo, GovernanceRuntimeError>;

    /// Create a single-owner DFLT(1) governance **overflow** record owned by the
    /// HKDF-derived `owner_keypair` (string form, as produced by
    /// [`GovernanceRuntimeDeps::format_writer_keypair`]). The derived owner grants
    /// write authority on any device with no persisted keypair, but the returned
    /// record key is NOT re-derivable — veilid mixes a random encryption key into
    /// it and refuses to re-create an existing owner+schema record. The caller
    /// therefore persists the returned key in the primary subkey's `overflow_next`
    /// chain and reuses it via `open_dht_record`; create runs at most once per
    /// page. The caller writes the overflow page to subkey 0 via `set_dht_value`
    /// with the same writer.
    async fn create_overflow_record(
        &self,
        owner_keypair: String,
    ) -> Result<DhtRecordInfo, GovernanceRuntimeError>;

    /// Convert an Ed25519 `(public, secret)` byte pair into the string
    /// form that the adapter understands as `writer` for `set_dht_value`
    /// + persists in `CommunityState.slot_keypair`. Lives on the trait
    /// because the underlying construction needs `veilid_core::KeyPair`
    /// (forbidden in this crate per Invariant 2).
    fn format_writer_keypair(&self, ed_public: [u8; 32], ed_secret: [u8; 32]) -> String;

    async fn get_dht_value(
        &self,
        lease: LeaseId,
        subkey: u32,
        force_refresh: bool,
    ) -> Result<Option<Vec<u8>>, GovernanceRuntimeError>;

    /// Returns `Ok(None)` on success, `Ok(Some(stale_bytes))` if the
    /// network's view is newer (M9.5 write conflict per §Failure 4 in
    /// `governance.rs`).
    async fn set_dht_value(
        &self,
        lease: LeaseId,
        subkey: u32,
        value: Vec<u8>,
        writer: Option<String>,
    ) -> Result<Option<Vec<u8>>, GovernanceRuntimeError>;

    /// Per-subkey seq numbers from this node's local cache, indexed by
    /// subkey. `None` means never written; `Some(0)` means **written
    /// once** — veilid's first write to a subkey lands at seq 0.
    ///
    /// Both adapters used to flatten that into `Vec<u64>` with `None`
    /// mapped to `0`, and both callers then read `seq != 0` as
    /// "occupied". A member who had written exactly one entry was
    /// therefore invisible: skipped by the hydration scan, and uncounted
    /// when deciding whether a segment was full. `Option` makes the
    /// distinction unspellable-away.
    async fn inspect_dht_record_local_seqs(
        &self,
        lease: LeaseId,
    ) -> Result<Vec<Option<u64>>, GovernanceRuntimeError>;

    /// Network-authoritative inspect (Veilid `DHTReportScope::UpdateGet`)
    /// returning per-subkey seq numbers. Used during slot claim to confirm
    /// occupancy from the network (not just our cached view) before
    /// writing. Same `Option` semantics as
    /// [`Self::inspect_dht_record_local_seqs`].
    async fn inspect_dht_record_update_get_seqs(
        &self,
        lease: LeaseId,
    ) -> Result<Vec<Option<u64>>, GovernanceRuntimeError>;

    /// Network-authoritative inspect (`DHTReportScope::UpdateGet`) returning
    /// the subkey indices that currently hold a value. Unlike
    /// `inspect_dht_record_update_get_seqs`, this preserves the "no value"
    /// (`ValueSeqNum::NONE`) vs "value present at seq 0" distinction, so it
    /// can drive a sparse fetch of only populated subkeys on a cold join —
    /// a 255-subkey SMPL record otherwise needs 255 serial network
    /// `get_dht_value` round-trips, which blows past the UI's join timeout.
    async fn inspect_dht_record_present_subkeys(
        &self,
        lease: LeaseId,
    ) -> Result<Vec<u32>, GovernanceRuntimeError>;

    // ---------- MEK cache ----------

    /// Community and channel keys (plan D6): current and historical
    /// generations, by exact scope.
    fn keys(&self) -> std::sync::Arc<dyn rekindle_types::channel_keys::ChannelKeyProvider>;
    fn insert_community_mek(&self, community_id: &str, mek: MekSnapshot);
    fn insert_channel_mek(&self, community_id: &str, channel_id: &str, mek: MekSnapshot);

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

    fn spawn_history_catchup(&self, community_id: &str);

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

    /// Apply recovered member state for one community: writes
    /// `my_subkey_index` if missing, updates `my_role_ids` if the DHT
    /// view is richer, then persists both to SQLite so the next login
    /// can skip the recovery step. Matches the pre-Phase-23 semantics
    /// of the inline body.
    fn apply_recovered_member_state(&self, community_id: &str, subkey_index: u32, role_ids: &[u32]);

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

    /// Snapshot one community's channel records as `(channel id hex,
    /// record key)`, in unspecified order. The id keys the channel's lease
    /// in [`CommunityLeases::channels`].
    fn channel_log_keys_for_community(&self, community_id: &str) -> Vec<(String, String)>;

    /// Active invite-secrets DFLT record keys this identity created, across
    /// every joined community — non-expired, non-empty. Driven by
    /// `dht_hydration::republish_active_records` to keep our own
    /// invites alive: the DFLT owner keypair is discarded after publish, so
    /// only a rehydrating re-open keeps the record on the network. Scoped to
    /// invites we authored because only those are local-store hits.
    fn list_my_active_invite_secret_keys(&self) -> Vec<String>;

    /// Merge `keys` into the community's
    /// `CommunityRecords.governance_overflow_keys` inventory. Idempotent
    /// (de-duped). Their leases reach the host separately, through
    /// [`Self::community_records_ready`]. The single
    /// entry point that registers a GovernanceOverflow record into the
    /// authoritative per-community inventory, from any producer: the write
    /// path (author's own spill pages) and every read path (Mutual Aid §14.1
    /// — a reader keeps alive every overflow record it follows).
    fn register_governance_overflow_keys(&self, community_id: &str, keys: &[String]);

    /// Snapshot the GovernanceOverflow record keys held in this community's
    /// `CommunityRecords.governance_overflow_keys` inventory. Consumed by
    /// keepalive warming (§14.1), the open+track orchestrators (§10), and
    /// login rehydration (D5) so overflow records share the same durability
    /// path as channels.
    fn governance_overflow_keys_for_community(&self, community_id: &str) -> Vec<String>;

    /// Apply the post-merge result for one community:
    /// 1. raise the governance clock to `max(current, accepted_clock)`
    ///    (`accepted_clock` from `merge_with_accepted`: what the accepted
    ///    entries justify, never a forged or rejected value),
    /// 2. install the new `GovernanceState` via `set_governance_state`,
    /// 3. persist the merged snapshot to SQLite so it survives restarts.
    ///
    /// Async because the SQLite persist is async. Returns when the
    /// in-memory + on-disk state are both updated.
    async fn apply_governance_rebuild_result(
        &self,
        community_id: &str,
        gov_state: GovernanceState,
        accepted_clock: u64,
    );

    /// Persist the raw per-author governance entry set for a community to
    /// local storage so `GovernanceState` can be losslessly re-merged on
    /// the next login — without waiting for the slow, best-effort DHT
    /// rebuild. The DHT stays authoritative; this is a warm cache
    /// refreshed on every successful rebuild. The full per-author grouping
    /// is preserved (not a flattened snapshot) so `merge`'s genesis +
    /// reader-validation rules reproduce an identical state. Fire-and-
    /// forget — failures are logged, never fatal.
    fn persist_governance_entries_cache(
        &self,
        community_id: &str,
        entries: &[(rekindle_types::id::PseudonymKey, Vec<GovernanceEntry>)],
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
