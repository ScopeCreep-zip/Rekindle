//! Deps trait surface for Phase 18 community lifecycle ops.
//!
//! One composite trait (`GovernanceRuntimeDeps`) abstracts every
//! src-tauri/Veilid/SQLite/Stronghold capability the crate's pure
//! orchestration logic needs. The adapter at
//! `src-tauri/src/services/governance_adapter.rs` implements it.
//!
//! Schwarzschild boundary: all Veilid types (`RecordKey`, `KeyPair`,
//! `RoutingContext`) are exchanged as opaque `String` / `Vec<u8>` here.
//! The crate never imports `veilid-core`.

use async_trait::async_trait;
use rekindle_codec::community::envelope::CommunityEnvelope;
use rekindle_governance::state::GovernanceState;
use rekindle_records::lease::{CommunityLeases, LeaseId};
use rekindle_types::id::PseudonymKey;

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
// attacker can reach. The alternative — hand-writing a 58-method double
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

    /// The host's wall clock (plan C9.2): every timestamp this crate writes
    /// or compares, so a simulated run fixes them. `Arc`, as `scope`, so the
    /// mock can return one.
    fn clock(&self) -> std::sync::Arc<dyn rekindle_utils::Clock>;

    /// The host's generator for the ids and keys this crate mints (plan
    /// C9.2). AEAD nonces never come from it.
    fn rng(&self) -> std::sync::Arc<dyn rekindle_utils::RandomSource>;

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
    /// a later read-only borrow never downgrades it. `intent` is the
    /// borrow's role ([`rekindle_records::port::OpenIntent`]): `Hold` for
    /// the community's session leases and our own slots, `Check` for
    /// probes and one-shot reads.
    async fn acquire_record(
        &self,
        record_key: &str,
        writer: Option<String>,
        intent: rekindle_records::port::OpenIntent,
    ) -> Result<LeaseId, GovernanceRuntimeError>;

    /// End a borrow; the record closes when it was the last.
    async fn release_record(&self, lease: LeaseId);

    /// Hand the community's session leases to the host. The host merges
    /// them into the community's held set (a lease on a record the set
    /// already holds is released, so repeat hand-overs do not accumulate),
    /// watches the records that were added, and starts its per-community
    /// loops (inspect, keepalive, presence poll) the first time, in its own
    /// scope. Called by origin, hydration and a new channel.
    async fn community_records_ready(&self, community_id: &str, leases: CommunityLeases);

    /// Create a record of `schema` with a fresh random owner; the returned
    /// `owner_keypair` is that owner's keypair text (plan E3.2: the anchor
    /// `A`, whose `K_A` origin zeroizes after one write, and `M(p)`).
    async fn create_record(
        &self,
        schema: &rekindle_records::port::RecordSchema,
    ) -> Result<DhtRecordInfo, GovernanceRuntimeError>;

    /// The owner public-key text of the record `lease` borrows, as its
    /// descriptor names it (`RecordPort::owner_of`): the pin of `A` to the
    /// cert's `K_A` and of `M(p)` to its `memberRecordOwner`.
    fn owner_of(&self, lease: LeaseId) -> Result<String, GovernanceRuntimeError>;
    /// The record port our member record is written and read through
    /// (`crate::member_card`).
    fn record_port(
        &self,
    ) -> Result<std::sync::Arc<dyn rekindle_records::port::RecordPort>, GovernanceRuntimeError>;

    async fn create_smpl_record(
        &self,
        member_pubkeys: &[[u8; 32]],
    ) -> Result<DhtRecordInfo, GovernanceRuntimeError>;

    /// Create a single-owner DFLT(1) record for one invite's encrypted
    /// `InviteSecrets` blob. `owner_keypair` is `Some` (DFLT owns subkey 0)
    /// so the caller can write the blob via `set_dht_value`.
    async fn create_dflt_record(&self) -> Result<DhtRecordInfo, GovernanceRuntimeError>;

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

    /// The hinted read of `subkey` (VU3's hint contract, `RecordPort::
    /// read_hinted`): a subkey a ring change or a hint named, forced from
    /// the network with no inspect first. The seq and data read, `None`
    /// when the read found nothing.
    async fn read_hinted_value(
        &self,
        lease: LeaseId,
        subkey: u32,
    ) -> Result<Option<(u64, Vec<u8>)>, GovernanceRuntimeError>;

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
    /// the join bootstrap-fetch path).
    async fn app_call_peer(
        &self,
        target_route_blob: &[u8],
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, GovernanceRuntimeError>;

    // ---------- DHT-hydration deps (Phase 23.C chiral split) ----------

    /// Snapshot every joined community's `(community_id, governance_key)`.
    /// Communities without a `governance_key` (v1.0 legacy) are skipped.
    /// Used by `dht_hydration::rebuild_governance` to enumerate
    /// records to merge.
    fn list_community_governance_targets(&self) -> Vec<(String, String)>;

    /// Snapshot every joined community's
    /// `(community_id, registry_key, my_pseudonym_hex_opt)`. Communities
    /// without a `member_registry_key` are skipped. Used by
    /// `dht_hydration::hydrate_one_community` to enumerate
    /// records to read from the DHT.
    fn list_registries_with_my_pseudonym(&self) -> Vec<(String, String, Option<String>)>;

    /// Apply recovered member state for one community: adopts
    /// `my_subkey_index` and persists it to SQLite so the next login can
    /// skip the recovery step. Our roles are the merged role assignments
    /// (the projection), never recovered here.
    fn apply_recovered_member_state(&self, community_id: &str, subkey_index: u32);

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
    /// Used by the Host's per-community login stage to drive
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

    /// Apply the post-merge result for one community: install the new
    /// `GovernanceState` via `set_governance_state` and persist the merged
    /// snapshot to SQLite so it survives restarts. Returns when the
    /// in-memory and on-disk state are both updated.
    async fn apply_governance_rebuild_result(&self, community_id: &str, gov_state: GovernanceState);

    /// The lock that serializes `community_id`'s governance rebuilds (plan
    /// E3.2 S7, D15: the state is a pure function of the op set, which only
    /// grows): a rebuild reads the op set and then awaits its dep fetches,
    /// so an older rebuild finishing last installed a state over a newer
    /// one. Held from the op-set read to the install.
    fn rebuild_lock(&self, community_id: &str) -> std::sync::Arc<tokio::sync::Mutex<()>>;

    /// `community_id`'s rebuild generations, which coalesce the triggers
    /// that arrive while a rebuild runs into one more (plan E3.2 S7l).
    fn rebuild_flight(
        &self,
        community_id: &str,
    ) -> std::sync::Arc<crate::rebuild_flight::RebuildFlight>;

    /// `community_id`'s kept op verdicts (plan E3.2 S7, R4-5): every read
    /// of its op set verifies only the ops it has not verified before.
    /// Created on first use, dropped with the community.
    fn op_verify_cache(
        &self,
        community_id: &str,
    ) -> std::sync::Arc<crate::verify_cache::OpVerifyCache>;

    // ---------- Governance ops (plan E3.2) ----------

    /// The community's pinned `GenesisCert`: `communities.genesis_cert`,
    /// decoded (`rekindle_codec::governance_op::genesis::decode_genesis`).
    /// Origin stores it with the row; S6 pins it at join
    /// ([`crate::genesis::read_pinned`]).
    fn genesis_cert(&self, community_id: &str) -> Option<rekindle_types::governance::GenesisCert>;
    /// The display name on `pseudonym`'s verified member card (`M`/0), when
    /// we hold one; never a field from an unverified peer payload.
    fn member_card_name(&self, community_id: &str, pseudonym: &PseudonymKey) -> Option<String>;
    /// Remember the display name on `pseudonym`'s card, which
    /// `crate::member_card::read_card` verified, or on our own card, which
    /// says `identity_display_name` (`member_card::write_card`).
    fn note_member_card_name(&self, community_id: &str, pseudonym: &PseudonymKey, name: String);
    /// Our pseudonym in the community, `None` while we hold none.
    fn my_pseudonym(&self, community_id: &str) -> Option<PseudonymKey>;
    /// Every op `governance_ops` holds for the community.
    async fn governance_ops(
        &self,
        community_id: &str,
    ) -> Result<Vec<crate::op_log::HeldOp>, GovernanceRuntimeError>;
    /// Store an op (an op already held keeps its row) and advance its
    /// author's `governance_heads` row.
    async fn store_governance_op(
        &self,
        community_id: &str,
        op: &crate::op_log::HeldOp,
    ) -> Result<(), GovernanceRuntimeError>;
    /// Record each op's fate in the last merge.
    async fn set_governance_op_fates(
        &self,
        community_id: &str,
        fates: Vec<crate::op_log::OpVerdict>,
    ) -> Result<(), GovernanceRuntimeError>;
    /// Write each checkpoint digest's state after the last merge
    /// (`governance_checkpoints`, plan E3.2 S7c.6); a kept snapshot stays.
    async fn record_governance_checkpoints(
        &self,
        community_id: &str,
        facts: Vec<crate::checkpoint_rows::CheckpointFact>,
    ) -> Result<(), GovernanceRuntimeError>;
    /// The snapshot kept for the checkpoint at `height` with `digest`, only
    /// while its row is usable.
    async fn usable_checkpoint_snapshot(
        &self,
        community_id: &str,
        height: u64,
        digest: [u8; 32],
    ) -> Result<Option<Vec<u8>>, GovernanceRuntimeError>;
    /// Keep `snapshot`, verified against its checkpoint's `stateRoot`, on
    /// that checkpoint's row when the row is usable.
    async fn keep_checkpoint_snapshot(
        &self,
        community_id: &str,
        height: u64,
        digest: [u8; 32],
        snapshot: Vec<u8>,
    ) -> Result<(), GovernanceRuntimeError>;
    /// The hint `community_id` was joined from while its backfill has not
    /// ended (`governance_partial`, plan E3.2 S7c.8 (1)); `None` once the
    /// community is whole.
    async fn governance_partial(
        &self,
        community_id: &str,
    ) -> Result<Option<crate::partial::PartialJoin>, GovernanceRuntimeError>;
    /// Mark `community_id` partial from `hint`, or whole with `None`.
    async fn set_governance_partial(
        &self,
        community_id: &str,
        hint: Option<crate::partial::PartialJoin>,
    ) -> Result<(), GovernanceRuntimeError>;
    /// The encoded op `op_ref` names, read from its author's op log
    /// (`rekindle_records::chain::read_log_after` on
    /// `op_ref.author_record`/`GOV_HEAD`, chain seq `author_seq + 1`),
    /// through `logs` ([`WalkLogCache::entry`](crate::op_walk::WalkLogCache::entry)):
    /// one read per author log per walk, rebuild or sweep (plan S7c.1).
    async fn fetch_governance_op(
        &self,
        op_ref: &rekindle_types::governance::OpRef,
        logs: &crate::op_walk::WalkLogCache,
    ) -> Result<Option<Vec<u8>>, GovernanceRuntimeError>;
    /// Queue our encoded op on the governance-op outbox stream
    /// (`StreamKind::GovernanceOp`, D22) and wait for what `landing` names. Its
    /// sender appends the row to our member record's op log with
    /// `chain::append_at(M(p), GOV_HEAD, K_M(p))`, `K_M(p)` from
    /// `VaultKey::MemberRecordOwner{community}` (origin stores it before the
    /// row; S6 at join), then writes our `Frontier` (`M`/3).
    ///
    /// The op is applied and gossiped before this runs and nothing cancels
    /// it, so every wait but the invite link's answers with an
    /// [`OpOutcome`] (plan E3.2 S6b; RV4 review M1, Matrix's local echo):
    /// [`OpLanding::FirstAttempt`] waits for the first attempt, at most
    /// `GOVERNANCE_ANSWER_BOUND`; [`OpLanding::Queued`] only for the row.
    /// A `Pending` op is announced (`GovernanceEvent::OpPublication`) when it
    /// lands. [`OpLanding::InviteLanded`] (the invite link, S6 (4)) waits through
    /// the retries up to the command's deadline and is
    /// [`GovernanceRuntimeError::NotYetPublished`] if it has not landed.
    async fn publish_governance_op(
        &self,
        community_id: &str,
        encoded: Vec<u8>,
        landing: OpLanding,
    ) -> Result<OpOutcome, GovernanceRuntimeError>;

    /// Fire-and-forget spawn of a text-MEK rotation task for a newly
    /// observed ban. Delegates to the existing
    /// `services::community::rotate_text_mek_for_departure`. The
    /// orchestrator iterates `new_bans` and calls this once per banned
    /// pseudonym so the rotation work is parallelised across bans.
    fn spawn_text_mek_rotation_for_ban(&self, community_id: &str, banned_pseudonym_hex: &str);

    /// The seq of `target_hex`'s slot value in each channel record of its
    /// segment, as we hold it now: a `MemberRemoved` records them for its
    /// target, our own `MemberLeft` for us, and readers keep the departed
    /// member's backfilled posts only at or below them, none from a record
    /// with no head (A/B finding R3-8). A record where we hold no value of
    /// the slot gets no head.
    ///
    /// # Errors
    /// Our own slot (`target_hex` is us) is unknown, or one of its records
    /// cannot be read: our leave would sign away our posts in it.
    async fn departed_slot_heads(
        &self,
        community_id: &str,
        target_hex: &str,
    ) -> Result<Vec<rekindle_types::governance::SlotHead>, GovernanceRuntimeError>;

    // ---------- Role mutations (Phase 23.D.15) ----------

    /// Snapshot of an existing role's current definition. Used by
    /// `edit_role` to compute the merged-update RoleDefinition.
    fn role_current_definition(
        &self,
        community_id: &str,
        role_id: u32,
    ) -> Option<crate::roles::RoleSnapshotInsert>;

    /// Return `(existing_ids, next_position)` for `create_role` —
    /// used to allocate a unique random role id and assign the next
    /// position slot.
    fn role_table_summary(&self, community_id: &str) -> (Vec<u32>, u32);

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
}

/// What the publisher of a governance op waits for
/// ([`GovernanceRuntimeDeps::publish_governance_op`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpLanding {
    /// The first attempt, at most `GOVERNANCE_ANSWER_BOUND`: a queued op
    /// stands, the outbox lands it, and the answer is `Pending`.
    FirstAttempt,
    /// The op on `M`/2 and our frontier on `M`/3, through the retries: only
    /// for what a joiner must find on the DHT before we hand it out, an
    /// invite's `InviteCreated` (S6 (4)). Stored in one transaction with a
    /// revoke of `invite_id` that the outbox writes once the op landed
    /// unless the answer handed the link out (review RV4 M3).
    InviteLanded { invite_id: [u8; 16] },
    /// Queued on the stream, not waited for: the outbox lands it behind our
    /// earlier ops (one stream per author, so the order holds).
    Queued,
}

/// How a published governance op stands when its command answers (plan
/// E3.2 S6b): both are successes, since the op is applied, gossiped and
/// queued and nothing cancels it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpOutcome {
    /// On our `M`/2 with our frontier on `M`/3.
    Landed,
    /// Queued on the governance-op stream; the outbox lands it, and the
    /// backend announces the landing.
    Pending,
}
