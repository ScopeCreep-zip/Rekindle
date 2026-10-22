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

use super::state::{CommunityState, MemberRecord};

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

    /// Drop a community — on leave.
    pub fn remove(&self, community_id: &str) {
        self.map.write().remove(community_id);
    }

    /// Drop every community. Called when the session ends.
    pub fn clear(&self) {
        self.map.write().clear();
    }
}
