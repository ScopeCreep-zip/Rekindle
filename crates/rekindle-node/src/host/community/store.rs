//! The Host's one community store (plan E3.1 A1).
//!
//! [`CommunityMap`] is the one map of joined communities, keyed by
//! governance key: `HostStores::communities`, which the GUI's
//! `AppState.communities` is in Shell mode, so what either side writes the
//! other reads. [`CommunityStore`] is the daemon's typed view of it
//! (`DaemonContext::communities`): the governance cache, the leases, the
//! overflow chain, the observed roster and the per-member roles, all on the
//! one [`CommunityState`].

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use parking_lot::RwLock;
use rekindle_governance::state::GovernanceState;
use rekindle_records::lease::{CommunityLeases, LeaseId, Merged};
use rekindle_transport::session::CommunityMembership;
use rekindle_types::id::ThreadId;

use rekindle_db::repo::communities::snapshot::StoredGovernance;

use super::state::{CommunityState, MemberRecord};
use crate::host::adapters::governance::projection::{self, GovernanceProjected};

/// Every joined community, keyed by governance key.
pub type CommunityMap = Arc<RwLock<HashMap<String, CommunityState>>>;

/// A new, empty [`CommunityMap`].
#[must_use]
pub fn new_map() -> CommunityMap {
    Arc::new(RwLock::new(HashMap::new()))
}

/// The typed accessors over the Host's [`CommunityMap`].
#[derive(Debug, Clone)]
pub struct CommunityStore {
    map: CommunityMap,
}

impl CommunityStore {
    /// The store over `map` (the Host's `HostStores::communities`).
    #[must_use]
    pub fn new(map: CommunityMap) -> Self {
        Self { map }
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

    /// The community's merged governance as SQLite stores it, at our
    /// governance clock; `None` before its governance has merged or when
    /// the clock is beyond what SQLite holds.
    #[must_use]
    pub fn governance_snapshot(&self, community_id: &str) -> Option<StoredGovernance> {
        self.read(community_id, |c| {
            let clock = i64::try_from(c.governance_clock).ok()?;
            projection::snapshot(c, clock)
        })
        .flatten()
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

    /// Mark the community's loops (the record keepalive) started; `true`
    /// the first time per session (plan C7.8b).
    pub fn start_keepalive_once(&self, community_id: &str) -> bool {
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

    /// Take the community's leases, for release (leave).
    pub fn take_leases(&self, community_id: &str) -> CommunityLeases {
        self.map
            .write()
            .get_mut(community_id)
            .map(|c| std::mem::take(&mut c.leases))
            .unwrap_or_default()
    }

    /// The channel records the community holds open this session, and our
    /// slot writer for opening more.
    #[must_use]
    pub fn channel_records(&self, community_id: &str) -> (HashSet<String>, Option<String>) {
        self.read(community_id, |c| {
            (
                c.open_community_records
                    .channel_keys
                    .iter()
                    .cloned()
                    .collect(),
                c.slot_keypair.clone(),
            )
        })
        .unwrap_or_default()
    }

    /// Record channel records opened this session.
    pub fn note_channel_records(&self, community_id: &str, keys: Vec<String>) {
        self.write(community_id, |c| {
            for key in keys {
                if !c.open_community_records.channel_keys.contains(&key) {
                    c.open_community_records.channel_keys.push(key);
                }
            }
        });
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

    /// How many members the community knows.
    #[must_use]
    pub fn member_count(&self, community_id: &str) -> usize {
        self.read(community_id, |c| c.known_members.len())
            .unwrap_or(0)
    }

    /// Replace the merged per-member role map.
    pub fn set_member_roles(&self, community_id: &str, roles: HashMap<String, Vec<u32>>) {
        self.write(community_id, |c| c.member_roles = roles);
    }

    /// Snapshot the merged per-member role map.
    #[must_use]
    pub fn member_roles(&self, community_id: &str) -> HashMap<String, Vec<u32>> {
        self.read(community_id, |c| c.member_roles.clone())
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
    /// the owner keypairs a created community has, so the governance
    /// adapter's readers find the community in the store the moment the
    /// create or join records it. Merges into an entry an earlier write
    /// (the governance cache) made.
    pub fn install_membership(
        &self,
        membership: &CommunityMembership,
        dht_owner_keypair: Option<&str>,
        registry_owner_keypair: Option<&str>,
    ) {
        self.write(&membership.governance_key, |c| {
            c.name.clone_from(&membership.community_name);
            c.my_pseudonym_key = Some(membership.pseudonym_key.clone());
            c.my_role_ids.clone_from(&membership.role_ids);
            c.member_registry_key =
                (!membership.registry_key.is_empty()).then(|| membership.registry_key.clone());
            c.my_subkey_index = Some(membership.slot_index);
            c.my_segment_index = membership.segment_index;
            c.slot_seed = membership.slot_seed.as_ref().map(hex::encode);
            c.governance_clock = c.governance_clock.max(membership.lamport_counter);
            c.mek_generation = membership.mek_generation;
            if let Some(keypair) = dht_owner_keypair {
                c.dht_owner_keypair = Some(keypair.to_string());
            }
            if let Some(keypair) = registry_owner_keypair {
                c.registry_owner_keypair = Some(keypair.to_string());
            }
            for (channel_id, record_key) in &membership.channel_record_keys {
                note_record_key(c, channel_id, record_key);
            }
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

    /// Our role ids, as the session changed them.
    pub fn set_my_role_ids(&self, community_id: &str, role_ids: &[u32]) {
        self.update(community_id, |c| c.my_role_ids = role_ids.to_vec());
    }

    /// The slot index and role ids the registry knows for us.
    pub fn set_slot_and_roles(&self, community_id: &str, slot_index: u32, role_ids: &[u32]) {
        self.update(community_id, |c| {
            c.my_subkey_index = Some(slot_index);
            c.my_role_ids = role_ids.to_vec();
        });
    }

    /// The MEK generation and member registry a JoinAccepted carried.
    pub fn set_mek_generation_and_registry(
        &self,
        community_id: &str,
        mek_generation: u64,
        registry_key: Option<&str>,
    ) {
        self.update(community_id, |c| {
            c.mek_generation = mek_generation;
            if let Some(key) = registry_key {
                c.member_registry_key = Some(key.to_string());
            }
        });
    }

    /// Our slot index from a member list, when we have none; `true` when
    /// it was taken.
    pub fn backfill_my_subkey_index(&self, community_id: &str, subkey_index: u32) -> bool {
        let mut map = self.map.write();
        match map.get_mut(community_id) {
            Some(c) if c.my_subkey_index.is_none() => {
                c.my_subkey_index = Some(subkey_index);
                true
            }
            _ => false,
        }
    }

    /// Install the material a keypair or slot-seed grant unwrapped.
    pub fn apply_slot_grant(
        &self,
        community_id: &str,
        slot_seed_hex: Option<&str>,
        slot_keypair: Option<&str>,
        dht_owner_keypair: Option<&str>,
        my_subkey_index: Option<u32>,
    ) {
        self.update(community_id, |c| {
            if let Some(seed) = slot_seed_hex {
                c.slot_seed = Some(seed.to_string());
            }
            if let Some(keypair) = slot_keypair {
                c.slot_keypair = Some(keypair.to_string());
            }
            if let Some(keypair) = dht_owner_keypair {
                c.dht_owner_keypair = Some(keypair.to_string());
            }
            if let Some(index) = my_subkey_index {
                c.my_subkey_index = Some(index);
            }
        });
    }

    /// A member finished onboarding with `role_ids`; ours when `is_self`.
    pub fn note_onboarding_completion(
        &self,
        community_id: &str,
        pseudonym_hex: &str,
        role_ids: &[u32],
        is_self: bool,
    ) {
        self.update(community_id, |c| {
            c.member_roles
                .insert(pseudonym_hex.to_string(), role_ids.to_vec());
            if is_self {
                c.onboarding_complete = true;
            }
        });
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

    /// A channel's record key merged governance gave it, onto the channel
    /// the store holds (insert-only, as the session records it).
    pub fn note_channel_record_key(&self, community_id: &str, channel_id: &str, record_key: &str) {
        self.update(community_id, |c| note_record_key(c, channel_id, record_key));
    }

    /// Drop a community — on leave.
    pub fn remove(&self, community_id: &str) {
        self.map.write().remove(community_id);
    }

    /// Drop every community. Called when the session ends.
    pub fn clear(&self) {
        self.map.write().clear();
    }
}

/// Set a channel's record key when the store holds the channel without one.
fn note_record_key(state: &mut CommunityState, channel_id: &str, record_key: &str) {
    if let Some(channel) = state.channels.iter_mut().find(|c| c.id == channel_id) {
        if channel.message_record_key.is_none() {
            channel.message_record_key = Some(record_key.to_string());
        }
    }
}
