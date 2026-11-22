//! The [`GovernanceRuntimeDeps`] trait itself; its vocabulary is in
//! [`super::types`], its role table in [`super::roles`], its op store in
//! [`super::op_store`].

use std::sync::Arc;

use async_trait::async_trait;
use rekindle_codec::community::envelope::CommunityEnvelope;
use rekindle_governance::state::GovernanceState;
use rekindle_records::lease::{CommunityLeases, LeaseId};
use rekindle_types::id::PseudonymKey;

use super::op_store::GovernanceOpStore;
use super::roles::RoleTable;
use super::slot_recovery::SlotRecovery;
use super::types::{
    CardText, CommunityDhtOpenSetup, CommunityInsert, CommunityMembership, DhtRecordInfo,
    DiscoveredMember, MekSnapshot, OnlineMemberSnapshot, OpLanding, OpOutcome, UserStatusKind,
};
use crate::error::GovernanceRuntimeError;
use crate::event::GovernanceRuntimeEvent;
use crate::rebuild_flight::RebuildFlight;
use crate::successor::SuccessorJudgements;
use crate::verify_cache::OpVerifyCache;

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
    /// This device's op key secret for `community_id` from its device
    /// vault (ADR 0022 rules 1, 4), or `None` when it holds none.
    ///
    /// # Errors
    /// Locked, or the device vault cannot be read.
    fn device_op_key(
        &self,
        community_id: &str,
    ) -> Result<Option<zeroize::Zeroizing<[u8; 32]>>, GovernanceRuntimeError>;
    /// Store this device's op key secret for `community_id`, the entry
    /// then holding the key alone: no `M(p,d)` bound, the member's first
    /// device's entry (ADR 0022 rule 3).
    ///
    /// # Errors
    /// Locked, or the device vault cannot be written.
    fn store_device_op_key(
        &self,
        community_id: &str,
        secret: &[u8; 32],
    ) -> Result<(), GovernanceRuntimeError>;
    /// Bind `record` (`M(p,d)`) and its owner keypair text `owner`
    /// (`K_M(p,d)`) to this device's held key for `community_id` (ADR 0022
    /// rules 3 and 4): an enrolled device's op record.
    ///
    /// # Errors
    /// Locked, no key held, or the device vault cannot be written.
    fn store_device_record(
        &self,
        community_id: &str,
        record: &str,
        owner: &str,
    ) -> Result<(), GovernanceRuntimeError>;
    fn identity_status(&self) -> UserStatusKind;
    fn our_route_blob(&self) -> Vec<u8>;

    // ---------- Community state (read) ----------

    fn community_membership(&self, community_id: &str) -> Option<CommunityMembership>;
    fn governance_state(&self, community_id: &str) -> Option<GovernanceState>;
    fn online_members(&self, community_id: &str) -> Vec<OnlineMemberSnapshot>;

    // ---------- Community state (mutation) ----------

    fn set_governance_state(&self, community_id: &str, state: GovernanceState);
    fn insert_community(&self, community: CommunityInsert);
    /// The next `seq` of our beat in `community_id`: `communities.beat_seq`
    /// advanced by one and persisted before it is returned, so a beat
    /// number is never reused across restarts (plan E3.4 S1).
    async fn next_beat_seq(&self, community_id: &str) -> Result<u64, GovernanceRuntimeError>;

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

    /// Hand the community's session leases to the host, which merges them
    /// into the held set (a repeat lease is released), watches the added
    /// records and starts the per-community loops the first time. Called by
    /// origin, hydration, a new channel, a segment expansion and overflow.
    async fn community_records_ready(&self, community_id: &str, leases: CommunityLeases);

    /// Undo a create that failed after origin installed the community:
    /// forget its keys, MEK, store entry and rows (nothing is told).
    async fn discard_created_community(&self, community_id: &str);

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
    /// the join bootstrap-fetch and Plate Gate expand-request paths).
    async fn app_call_peer(
        &self,
        target_route_blob: &[u8],
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, GovernanceRuntimeError>;

    // ---------- DHT-hydration deps (Phase 23.C chiral split) ----------

    /// Snapshot every joined community's `(community_id, anchor)`.
    /// Communities without a `anchor` (v1.0 legacy) are skipped.
    /// Used by `dht_hydration::rebuild_governance` to enumerate
    /// records to merge.
    fn list_community_governance_targets(&self) -> Vec<(String, String)>;

    /// Snapshot every joined community's open-DHT-records setup info.
    /// Used by the Host's per-community login stage to drive
    /// the per-community `open_dht_record` calls without holding the
    /// `state.communities` read-guard across `.await`.
    fn list_communities_for_dht_open(&self) -> Vec<CommunityDhtOpenSetup>;

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

    /// The lock that serializes `community_id`'s bootstrap admissions (plan
    /// SU1): from the merge that admits a join through its store and, when
    /// it fills its invite's `max_uses`, the invite's revoke, so two
    /// joiners never both take the last use. Never held by a rebuild.
    fn admission_lock(&self, community_id: &str) -> std::sync::Arc<tokio::sync::Mutex<()>>;

    /// `community_id`'s rebuild generations, which coalesce the triggers
    /// that arrive while a rebuild runs into one more (plan E3.2 S7l).
    fn rebuild_flight(&self, community_id: &str) -> Arc<RebuildFlight>;

    /// `community_id`'s kept op verdicts (plan E3.2 S7, R4-5): every read
    /// of its op set verifies only the ops it has not verified before.
    /// Created on first use, dropped with the community.
    fn op_verify_cache(&self, community_id: &str) -> Arc<OpVerifyCache>;

    /// `community_id`'s judged successors (plan E3.2 SS2, lead decision
    /// (31)): each applied successor's far cert is read until it is offered
    /// or refused. Kept for the session, dropped with the community.
    fn successor_judgements(&self, community_id: &str) -> Arc<SuccessorJudgements>;

    // ---------- Governance ops (plan E3.2) ----------

    /// The community's pinned `GenesisCert`: `communities.genesis_cert`,
    /// decoded (`rekindle_codec::governance_op::genesis::decode_genesis`).
    /// Origin stores it with the row; S6 pins it at join
    /// ([`crate::genesis::read_pinned`]).
    fn genesis_cert(&self, community_id: &str) -> Option<rekindle_types::governance::GenesisCert>;
    /// The display name on `pseudonym`'s verified member card (`M`/0), when
    /// we hold one and it names one; never a field from an unverified peer
    /// payload.
    fn member_card_name(&self, community_id: &str, pseudonym: &PseudonymKey) -> Option<String>;
    /// Remember what `pseudonym`'s card says, which
    /// `crate::member_card::read_card` verified (plan E3.4 S6: the card
    /// cache is written only from a verified read).
    fn note_member_card(&self, community_id: &str, pseudonym: &PseudonymKey, card: CardText);
    /// Our pseudonym in the community, `None` while we hold none.
    fn my_pseudonym(&self, community_id: &str) -> Option<PseudonymKey>;
    /// Keep `checkpoint`'s sealed directory pages warm (plan E3.2 S7
    /// record warming): called with the latest endorsed checkpoint after
    /// every merge, and with our own proposal once its pages are sealed;
    /// a newer height releases the pages of an older one.
    fn warm_checkpoint_pages(
        &self,
        community_id: &str,
        checkpoint: &rekindle_types::governance::Checkpoint,
    );
    /// Gossip our stored op (`governanceOp @74`) before its install, which
    /// prunes a removed member that must still hear the op removing it.
    fn gossip_governance_op(&self, community_id: &str, encoded: &[u8]);

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

    /// Start [`crate::member_card::refresh_member_cards`] for
    /// `community_id` as a task on the session (plan JP1): a rebuild never
    /// waits on the card reads, which are DHT round trips, so a rebuild a
    /// `BootstrapRequest` triggers answers within the app_call deadline. At
    /// most one refresh runs per community; a call while one runs costs
    /// one more turn after it, not a read per call. Its names arrive
    /// through `note_member_card_name` as before. Stopped with the session
    /// or the community.
    fn spawn_member_card_refresh(&self, community_id: &str);

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

    /// The store of the community's governance ops ([`GovernanceOpStore`]):
    /// held ops, fates, checkpoint rows and snapshots, the partial mark.
    fn op_store(&self) -> Arc<dyn GovernanceOpStore>;

    // ---------- Role mutations (Phase 23.D.15) ----------

    /// The role table the role commands read and write ([`RoleTable`]).
    fn roles(&self) -> Arc<dyn RoleTable>;

    /// Our slot and its keys, recovered at login (the slot model's
    /// hydration, `dht_hydration::hydrate_one_community`).
    fn slot_recovery(&self) -> Arc<dyn SlotRecovery>;
}
