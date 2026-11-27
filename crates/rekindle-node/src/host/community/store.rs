//! The Host's one community store (plan E3.1 A1).
//!
//! [`CommunityMap`] is the one map of joined communities, keyed by
//! anchor key: `HostStores::communities`, which the GUI's
//! `AppState.communities` is in Shell mode, so what either side writes the
//! other reads. [`CommunityStore`] is the daemon's typed view of it
//! (`DaemonContext::communities`): the governance cache, the leases, the
//! overflow chain, the observed roster and the per-member roles, all on the
//! one [`CommunityState`].

use rekindle_channel::held::HeldChanges;
use rekindle_governance_runtime::CardText;

use crate::host::adapters::channel::held_changes::HeldChange;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use locks::{CommunityLocks, FollowUp};
use parking_lot::RwLock;
use rekindle_codec::capnp_codec::chain::PagePointer;
use rekindle_governance::state::GovernanceState;
use rekindle_governance_runtime::rebuild_flight::RebuildFlight;
use rekindle_governance_runtime::successor::SuccessorJudgements;
use rekindle_governance_runtime::verify_cache::OpVerifyCache;
use rekindle_protocol::dht::pool::keepalive::WarmRecord;
use rekindle_records::lease::{CommunityLeases, LeaseId, Merged};
use rekindle_transport::session::CommunityMembership;
use rekindle_types::id::ThreadId;

use rekindle_db::repo::communities::snapshot::StoredGovernance;

use super::state::{CommunityState, MemberRecord};
use crate::host::adapters::governance::projection::{self, GovernanceProjected};

/// Every joined community, keyed by anchor key.
pub type CommunityMap = Arc<RwLock<HashMap<String, CommunityState>>>;

/// A new, empty [`CommunityMap`].
mod cards;
mod locks;
mod removal;

#[must_use]
pub fn new_map() -> CommunityMap {
    Arc::new(RwLock::new(HashMap::new()))
}

/// The typed accessors over the Host's [`CommunityMap`].
#[derive(Debug, Clone)]
pub struct CommunityStore {
    map: CommunityMap,
    /// The display names on members' verified cards (`M`/0), by community
    /// and pseudonym (plan E3.2).
    member_cards: Arc<RwLock<BTreeMap<(String, [u8; 32]), CardText>>>,
    /// One lock per community serializing its watch-ring moves (plan E3.2
    /// S7): a move reads the merged member set and then awaits its opens
    /// and releases, so two concurrent moves (rebuilds from gossip, hints,
    /// the sweep) undid each other's assignments.
    ring_moves: CommunityLocks,
    /// One lock per community serializing its governance rebuilds (S7):
    /// an older rebuild finishing last installed over a newer one.
    rebuilds: CommunityLocks,
    /// One rebuild single flight per community (S7l): the triggers that
    /// arrive while a rebuild runs coalesce into one more.
    rebuild_flights: Arc<parking_lot::Mutex<BTreeMap<String, Arc<RebuildFlight>>>>,
    /// One community's kept op verdicts (S7, R4-5): an op verifies once
    /// per process, not on every read of the op set.
    op_verify: Arc<parking_lot::Mutex<BTreeMap<String, Arc<OpVerifyCache>>>>,
    /// One community's judged successors (E3.2 SS2, decision (31)).
    successor_judgements: Arc<parking_lot::Mutex<BTreeMap<String, Arc<SuccessorJudgements>>>>,
    /// One lock per community serializing its `Frontier` (`M`/3) writes
    /// (S7l): `append_own_op` and the post-rebuild republish were two
    /// unsynchronized writers, so an older head set could land last.
    frontier_writes: CommunityLocks,
    /// One lock per community serializing its bootstrap admissions (plan
    /// SU1): the check, the store and the invite's used-up revoke of one
    /// join never interleave with another's.
    admissions: CommunityLocks,
    /// Profile saves and game rewrites both modify our card.
    profile_writes: CommunityLocks,
    /// The post-rebuild follow-up per community (S7l): at most one runs,
    /// and the rebuilds that finish meanwhile only mark it dirty.
    follow_ups: Arc<parking_lot::Mutex<BTreeMap<String, FollowUp>>>,
    /// The member card refresh per community (plan JP1): at most one
    /// runs, and the rebuilds that finish meanwhile only mark it dirty.
    card_refreshes: Arc<parking_lot::Mutex<BTreeMap<String, FollowUp>>>,
    /// The channel edits and deletes that arrived before their message
    /// (plan E3.3 E5), bounded, keyed by author pseudonym and message id.
    held_changes: Arc<parking_lot::Mutex<HeldChanges<HeldChange>>>,
    /// Every community's unlock barrier and the governance writes it holds
    /// (plan E3.2 S7d-keys.8).
    barriers: Arc<rekindle_governance_runtime::unlock_barrier::UnlockBarriers>,
}

impl CommunityStore {
    /// The store over `map` (the Host's `HostStores::communities`).
    #[must_use]
    pub fn new(map: CommunityMap) -> Self {
        Self {
            map,
            member_cards: Arc::new(RwLock::new(BTreeMap::new())),
            ring_moves: Arc::default(),
            rebuilds: Arc::default(),
            rebuild_flights: Arc::default(),
            op_verify: Arc::default(),
            successor_judgements: Arc::default(),
            frontier_writes: Arc::default(),
            admissions: Arc::default(),
            profile_writes: Arc::default(),
            follow_ups: Arc::default(),
            card_refreshes: Arc::default(),
            held_changes: Arc::default(),
            barriers: Arc::default(),
        }
    }

    /// Every community's unlock barrier (plan E3.2 S7d-keys.8).
    #[must_use]
    pub fn unlock_barriers(&self) -> &rekindle_governance_runtime::unlock_barrier::UnlockBarriers {
        &self.barriers
    }

    /// The channel changes waiting for their message (plan E3.3 E5).
    #[must_use]
    pub(crate) fn held_changes(&self) -> &parking_lot::Mutex<HeldChanges<HeldChange>> {
        &self.held_changes
    }

    /// The shared map itself.
    #[must_use]
    pub fn map(&self) -> &CommunityMap {
        &self.map
    }

    fn read<T>(&self, community_id: &str, f: impl FnOnce(&CommunityState) -> T) -> Option<T> {
        self.map.read().get(community_id).map(f)
    }

    fn write<T>(&self, community_id: &str, f: impl FnOnce(&mut CommunityState) -> T) -> T {
        let mut map = self.map.write();
        let entry = map
            .entry(community_id.to_string())
            .or_insert_with(|| CommunityState::new(community_id));
        f(entry)
    }

    /// The community key generation the Host holds now, as the window view
    /// serves it (`CommunityView::mek_generation`); `None` for a community
    /// we do not know.
    #[must_use]
    pub fn mek_generation(&self, community_id: &str) -> Option<u64> {
        self.read(community_id, |c| c.mek_generation)
    }

    /// Cached governance state for a community, if it has been merged.
    #[must_use]
    pub fn governance_state(&self, community_id: &str) -> Option<GovernanceState> {
        self.read(community_id, |c| c.governance_state.clone())
            .flatten()
    }

    /// Replace the cached governance state, creating the entry if this is
    /// the first merge for the community.
    pub fn set_governance_state(&self, community_id: &str, state: GovernanceState) {
        self.write(community_id, |c| c.governance_state = Some(state));
    }

    /// Project merged governance onto the community (channels, roles,
    /// categories, our role ids, segments; plan E3.1 B2c) and install it,
    /// creating the entry if this is the first merge for the community.
    /// A thread new to us whose creator is timed out at `now_secs` (the
    /// clock port) is held from the state first, and stays held; the
    /// threads newly held come back with the projection, for the caller to
    /// store (`repo::held_threads`).
    pub fn project_governance(
        &self,
        community_id: &str,
        mut state: GovernanceState,
        now_secs: u64,
    ) -> (GovernanceProjected, Vec<ThreadId>) {
        self.write(community_id, |c| {
            let held = rekindle_channel::ingest::hold_timed_out_threads(
                c.governance_state.as_ref(),
                &mut c.held_threads,
                &mut state,
                now_secs,
            );
            (projection::project(c, state), held)
        })
    }

    /// The community's merged governance as SQLite stores it; `None`
    /// before its governance has merged.
    #[must_use]
    pub fn governance_snapshot(&self, community_id: &str) -> Option<StoredGovernance> {
        self.read(community_id, projection::snapshot).flatten()
    }

    /// Fold `incoming` into the community's leases
    /// ([`CommunityLeases::merge`]); the caller releases the returned
    /// surplus.
    pub fn hold_leases(
        &self,
        community_id: &str,
        incoming: CommunityLeases,
        key_of: impl Fn(LeaseId) -> Option<String>,
    ) -> Merged {
        self.write(community_id, |c| c.leases.merge(incoming, key_of))
    }

    /// Whether the community holds its registry lease.
    #[must_use]
    pub fn holds_registry(&self, community_id: &str) -> bool {
        self.read(community_id, |c| c.leases.registry.is_some())
            .unwrap_or(false)
    }

    /// Mark the community's loops (the record keepalive, the presence
    /// poll) started; `true` the first time per session (plan C7.8b).
    pub fn start_loops_once(&self, community_id: &str) -> bool {
        self.write(community_id, |c| {
            !std::mem::replace(&mut c.loops_started, true)
        })
    }

    /// Every lease the community holds (governance, registry, channels,
    /// segments, overflow), for watching them.
    #[must_use]
    pub fn held_leases(&self, community_id: &str) -> Vec<LeaseId> {
        self.read(community_id, |c| c.leases.all().collect())
            .unwrap_or_default()
    }

    /// Our own member record and our watch ring's, kept warm (plan E3.2
    /// S7 record warming).
    pub fn warm_member_records(&self, community_id: &str) -> Vec<String> {
        self.read(
            community_id,
            crate::host::adapters::governance::watch_ring::warm_member_records,
        )
        .unwrap_or_default()
    }

    /// The sealed pages the community keeps warm (plan E3.2 S7).
    pub fn warm_sealed_pages(&self, community_id: &str) -> Vec<WarmRecord> {
        self.read(community_id, super::sealed_pages::warm_sealed_pages)
            .unwrap_or_default()
    }

    /// Hold `checkpoint`'s directory pages, released by a newer one
    /// (`SealedPages::checkpoint`); `me` is our pseudonym there.
    pub fn warm_checkpoint_pages(
        &self,
        community_id: &str,
        checkpoint: &rekindle_types::governance::Checkpoint,
        me: Option<&rekindle_types::id::PseudonymKey>,
    ) {
        if let Some(c) = self.map.write().get_mut(community_id) {
            c.sealed_pages
                .checkpoint(checkpoint.height, &checkpoint.pages, &checkpoint.cut, me);
        }
    }

    /// One of our own op pages, sealed by an append (plan E3.2 S7).
    pub fn note_own_op_page(&self, community_id: &str, page: &PagePointer) {
        if let Some(c) = self.map.write().get_mut(community_id) {
            c.sealed_pages.own_page(page);
        }
    }

    /// The community's held leases but its anchor's, to watch: the anchor
    /// is immutable (plan E3.2 S7).
    pub fn watched_leases(&self, community_id: &str) -> Vec<LeaseId> {
        self.read(community_id, |c| {
            c.leases
                .all()
                .filter(|l| c.leases.anchor != Some(*l))
                .collect()
        })
        .unwrap_or_default()
    }

    /// Take the community's leases, for release (leave): its own, its
    /// watch ring's, its removal watch's (plan RM2) and its key drop
    /// watch's (plan RV2d), so a ring move that finished after the leave's
    /// `release_watch_ring` leaks no lease.
    pub fn take_leases(&self, community_id: &str) -> (CommunityLeases, Vec<LeaseId>) {
        self.map
            .write()
            .get_mut(community_id)
            .map(|c| {
                (
                    std::mem::take(&mut c.leases),
                    std::mem::take(&mut c.watch_ring)
                        .into_values()
                        .chain(std::mem::take(&mut c.removal_watch).into_values())
                        .chain(c.key_drop.take().map(|(_, lease)| lease))
                        .collect(),
                )
            })
            .unwrap_or_default()
    }

    /// The community's segment descriptors (empty for an unknown one).
    #[must_use]
    pub fn segments(&self, community_id: &str) -> Vec<rekindle_types::presence::SegmentDescriptor> {
        self.read(community_id, |c| c.segments.clone())
            .unwrap_or_default()
    }

    /// Governance overflow chain for the community, in order.
    #[must_use]
    pub fn overflow_keys(&self, community_id: &str) -> Vec<String> {
        self.read(community_id, |c| {
            c.open_community_records.governance_overflow_keys.clone()
        })
        .unwrap_or_default()
    }

    /// Append to the overflow chain, skipping keys already present so a
    /// repeated discovery pass does not grow the chain.
    pub fn register_overflow_keys(&self, community_id: &str, keys: &[String]) {
        self.write(community_id, |c| {
            let chain = &mut c.open_community_records.governance_overflow_keys;
            for key in keys {
                if !chain.contains(key) {
                    chain.push(key.clone());
                }
            }
        });
    }

    /// Replace the observed roster, and `known_members` with its keys.
    ///
    /// Wholesale rather than merged: the poll re-derives the full set each
    /// tick, so a member who vanished from the registry vanishes here too.
    pub fn set_members(&self, community_id: &str, members: HashMap<String, MemberRecord>) {
        self.write(community_id, |c| {
            c.known_members = members.keys().cloned().collect();
            c.members = members;
        });
    }

    /// Snapshot the observed roster.
    #[must_use]
    pub fn members(&self, community_id: &str) -> HashMap<String, MemberRecord> {
        self.read(community_id, |c| c.members.clone())
            .unwrap_or_default()
    }

    /// The community's known members (hex pseudonyms).
    #[must_use]
    pub fn known_members(&self, community_id: &str) -> HashSet<String> {
        self.read(community_id, |c| c.known_members.clone())
            .unwrap_or_default()
    }

    /// Add `candidates` to a joined community's known members and return
    /// the ones it did not know: a community not in the store learns none.
    pub fn extend_known_members(&self, community_id: &str, candidates: Vec<String>) -> Vec<String> {
        let mut map = self.map.write();
        let Some(c) = map.get_mut(community_id) else {
            return Vec::new();
        };
        candidates
            .into_iter()
            .filter(|key| c.known_members.insert(key.clone()))
            .collect()
    }

    /// Forget a departed member: off the known members, the roster and the
    /// gossip overlay.
    pub fn forget_member(&self, community_id: &str, pseudonym_hex: &str) {
        self.update(community_id, |c| {
            c.known_members.remove(pseudonym_hex);
            c.members.remove(pseudonym_hex);
            if let Some(gossip) = c.gossip.as_mut() {
                gossip.online_members.remove(pseudonym_hex);
                gossip.peers.remove(pseudonym_hex);
            }
        });
    }

    /// The communities with a member registry but no registry owner
    /// keypair in memory: the ones hydration recovers from the vault.
    #[must_use]
    pub fn missing_registry_keypairs(&self) -> Vec<String> {
        self.map
            .read()
            .iter()
            .filter(|(_, c)| c.member_registry_key.is_some() && c.registry_owner_keypair.is_none())
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Set a joined community's registry owner keypair; a community not in
    /// the store is left absent.
    pub fn set_registry_owner_keypair(&self, community_id: &str, keypair: String) {
        if let Some(c) = self.map.write().get_mut(community_id) {
            c.registry_owner_keypair = Some(keypair);
        }
    }

    /// How many members the merged governance counts (its `MemberJoined`
    /// set, plan E3.2), never the presence roster, which lists only the
    /// members a scan or gossip reached (A/B finding R1-7: 1 shown of 2).
    #[must_use]
    pub fn member_count(&self, community_id: &str) -> usize {
        self.read(community_id, |c| {
            c.governance_state
                .as_ref()
                .map_or(0, GovernanceState::member_count)
        })
        .unwrap_or(0)
    }

    /// Our role ids: the merged role assignments as the projection laid
    /// them down (`projection::project_my_roles`).
    #[must_use]
    pub fn my_role_ids(&self, community_id: &str) -> Vec<rekindle_types::id::RoleId> {
        self.read(community_id, |c| c.my_role_ids.clone())
            .unwrap_or_default()
    }

    /// Every member's role ids (pseudonym hex → ids sorted by their bytes), from
    /// the merged role assignments (plan E3.2, D15); never a cache.
    #[must_use]
    pub fn member_role_ids(
        &self,
        community_id: &str,
    ) -> HashMap<String, Vec<rekindle_types::id::RoleId>> {
        self.read(community_id, |c| {
            c.governance_state
                .as_ref()
                .map(crate::host::adapters::governance::projection::member_role_ids)
        })
        .flatten()
        .unwrap_or_default()
    }

    /// Record that a channel finished its initial history catch-up.
    pub fn mark_channel_synced(&self, community_id: &str, channel_id: &str) {
        self.write(community_id, |c| {
            c.synced_channels.insert(channel_id.to_string());
        });
    }

    /// Apply `f` to a joined community's entry; nothing when it is not in
    /// the store (a membership change for a community we left).
    fn update(&self, community_id: &str, f: impl FnOnce(&mut CommunityState)) {
        if let Some(entry) = self.map.write().get_mut(community_id) {
            f(entry);
        }
    }

    /// Install a recorded membership (plan E3.1 B2b): our pseudonym, slot,
    /// segment, slot seed, registry, roles, clock and MEK generation, and
    /// the registry owner keypair a created community has, so the governance
    /// adapter's readers find the community in the store the moment the
    /// create or join records it. Merges into an entry an earlier write
    /// (the governance cache) made.
    pub fn install_membership(
        &self,
        membership: &CommunityMembership,
        registry_owner_keypair: Option<&str>,
    ) {
        self.write(&membership.anchor, |c| {
            c.name.clone_from(&membership.community_name);
            c.my_pseudonym_key = Some(membership.pseudonym_key.clone());
            c.member_registry_key =
                (!membership.registry_key.is_empty()).then(|| membership.registry_key.clone());
            c.my_subkey_index = Some(membership.slot_index);
            c.my_segment_index = membership.segment_index;
            c.slot_seed = membership.slot_seed.as_ref().map(hex::encode);
            c.mek_generation = membership.mek_generation;
            if let Some(keypair) = registry_owner_keypair {
                c.registry_owner_keypair = Some(keypair.to_string());
            }
        });
    }

    /// Our own member record's key in `community_id`, as the join or create
    /// recorded it (plan E3.2, ADR 0011).
    pub fn set_my_member_record(&self, community_id: &str, record: &str) {
        self.update(community_id, |c| {
            c.my_member_record = Some(record.to_owned());
        });
    }

    /// Pin the community's `GenesisCert` (plan E3.2).
    pub fn set_genesis_cert(
        &self,
        community_id: &str,
        cert: rekindle_types::governance::GenesisCert,
    ) {
        self.update(community_id, |c| c.genesis_cert = Some(cert));
    }

    /// The community's pinned `GenesisCert`.
    #[must_use]
    pub fn genesis_cert(
        &self,
        community_id: &str,
    ) -> Option<rekindle_types::governance::GenesisCert> {
        self.map.read().get(community_id)?.genesis_cert.clone()
    }

    /// The slot index the registry knows for us.
    pub fn set_my_slot(&self, community_id: &str, slot_index: u32) {
        self.update(community_id, |c| c.my_subkey_index = Some(slot_index));
    }

    /// Count a merged join of `pseudonym_hex` at `now_secs` in the
    /// community's raid window (architecture §20.6), under the merged
    /// policy's thresholds: the alert when the rate trips them.
    pub fn observe_raid_join(
        &self,
        community_id: &str,
        pseudonym_hex: &str,
        now_secs: u64,
    ) -> Option<rekindle_governance::RaidAlert> {
        let mut map = self.map.write();
        let c = map.get_mut(community_id)?;
        let policy = (c.governance_state.as_ref()).and_then(|gs| gs.community_policy.as_ref());
        rekindle_governance::observe_join(
            &mut c.recent_member_joins,
            now_secs,
            pseudonym_hex,
            policy,
        )
    }

    /// A member finished onboarding; ours when `is_self`. Its roles are
    /// the merged role assignments, not stored here.
    pub fn note_onboarding_completion(&self, community_id: &str, is_self: bool) {
        if is_self {
            self.update(community_id, |c| c.onboarding_complete = true);
        }
    }

    /// The member registry key and our pseudonym.
    pub fn registry_and_pseudonym(&self, community_id: &str) -> (Option<String>, Option<String>) {
        self.read(community_id, |c| {
            (c.member_registry_key.clone(), c.my_pseudonym_key.clone())
        })
        .unwrap_or_default()
    }

    /// An online member seen in the registry: on the roster, and a gossip
    /// peer once it has a route. `false` when the community has no overlay.
    pub fn note_online_member(
        &self,
        community_id: &str,
        pseudonym_hex: &str,
        member: super::state::OnlineMember,
    ) -> bool {
        let mut map = self.map.write();
        let Some(gossip) = map.get_mut(community_id).and_then(|c| c.gossip.as_mut()) else {
            return false;
        };
        if !member.route_blob.is_empty() {
            gossip
                .peers
                .insert(pseudonym_hex.to_string(), member.clone());
        }
        gossip
            .online_members
            .insert(pseudonym_hex.to_string(), member);
        true
    }
}
