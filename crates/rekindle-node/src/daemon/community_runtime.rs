//! Per-community runtime state for the daemon's governance adapter.
//!
//! This is the daemon's *own* state, deliberately not shared with the
//! Tauri host. `docs/architecture/services-pattern.md` §2 defines an
//! adapter as implementing a `Deps` trait "against the live `AppState` +
//! `AppHandle` + `DbPool`" — the **Schwarzschild boundary**. The shared
//! contract between the two tracks is the *trait*, not the state behind
//! it, which is why `GovernanceRuntimeDeps` passes snapshots
//! (`MekSnapshot`, `OnlineMemberSnapshot`, an all-`Option`
//! `CommunityMembership`) rather than a state struct. Hoisting
//! `src-tauri`'s `CommunityState` into a shared crate would drag
//! Tauri-shaped, SQLite-backed state across that horizon.
//!
//! So: state shaped for the daemon. What lives here is only what is
//! **runtime-derived and not persisted** —
//!
//! - `GovernanceState` is the cached result of a CRDT merge over the
//!   governance record's subkeys. It is rebuilt from the DHT, so
//!   persisting it would only create a second source of truth that can
//!   go stale.
//! - Open-record keys track which DHT records this process currently
//!   holds open. That is per-process by definition.
//!
//! The persisted half — segment index, governance Lamport counter, MEK
//! generation — lives on `rekindle_transport::session::CommunityMembership`
//! in `session.json`, beside the rest of the membership.

use std::collections::{BTreeSet, HashMap};

use parking_lot::RwLock;
use rekindle_governance::state::GovernanceState;

/// Runtime (non-persisted) state for one community.
#[derive(Debug, Default)]
pub struct CommunityRuntime {
    /// Cached CRDT-merged governance state, or `None` before the first
    /// merge completes. Rebuilt from the DHT on start.
    pub governance: Option<GovernanceState>,
    /// DHT records this process holds open for the community —
    /// governance, registry, and each channel record.
    ///
    /// A `BTreeSet` rather than a `Vec`: `mark_open_channel_record` is
    /// called on every channel discovery pass, and the trait's
    /// `open_record_keys` is read on paths that then open each key.
    /// Duplicates there mean redundant `open_dht_record` calls.
    pub open_record_keys: BTreeSet<String>,
    /// Governance overflow record keys (the `overflow_next` chain), in
    /// chain order — so this one stays a `Vec`.
    pub overflow_keys: Vec<String>,
    /// The community's members, as the presence poll last observed
    /// them, keyed by hex pseudonym.
    ///
    /// Runtime-derived like everything else here: there is no
    /// membership ledger to persist. `communities-overview.md` calls
    /// the registry a presence directory — "Single struct overwritten
    /// per heartbeat. Never grows" — and under `AdmissionMode::Open`
    /// governance holds no entry for a joiner either. A member is a
    /// *predicate over registry rows*: a validly-signed
    /// `MemberPresence` whose author is neither banned nor departed.
    ///
    /// The poll evaluates that predicate once per tick and leaves the
    /// answer here, so queries read a materialised roster instead of
    /// re-verifying signatures per request. That is the whole reason
    /// the daemon needs a presence subsystem rather than a member-index
    /// read.
    pub members: HashMap<String, MemberRecord>,
    /// Per-member roles as merged from governance, keyed by hex
    /// pseudonym. Separate from `members` because a role assignment can
    /// arrive (via the CRDT) before the member's presence row does.
    pub member_roles: HashMap<String, Vec<u32>>,
    /// Channels whose initial history catch-up has completed.
    pub synced_channels: BTreeSet<String>,
}

/// One member as last seen in the registry.
#[derive(Debug, Clone, Default)]
pub struct MemberRecord {
    /// Peer-supplied. Untrusted display data — render escaped.
    pub display_name: Option<String>,
    pub subkey_index: u32,
    pub segment_index: u32,
    pub bio: Option<String>,
    pub pronouns: Option<String>,
    pub theme_color: Option<u32>,
    pub badges: Vec<String>,
    pub avatar_ref: Option<String>,
    pub banner_ref: Option<String>,
}

/// All communities' runtime state, keyed by governance key.
#[derive(Debug, Default)]
pub struct CommunityRuntimeMap {
    inner: RwLock<HashMap<String, CommunityRuntime>>,
}

impl CommunityRuntimeMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Cached governance state for a community, if it has been merged.
    pub fn governance_state(&self, community_id: &str) -> Option<GovernanceState> {
        self.inner.read().get(community_id)?.governance.clone()
    }

    /// Replace the cached governance state, creating the entry if this
    /// is the first merge for the community.
    pub fn set_governance_state(&self, community_id: &str, state: GovernanceState) {
        self.inner
            .write()
            .entry(community_id.to_string())
            .or_default()
            .governance = Some(state);
    }

    /// Records currently open for the community.
    pub fn open_record_keys(&self, community_id: &str) -> Vec<String> {
        self.inner
            .read()
            .get(community_id)
            .map(|c| c.open_record_keys.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Note a record as open. Idempotent.
    pub fn mark_open_record(&self, community_id: &str, record_key: String) {
        self.inner
            .write()
            .entry(community_id.to_string())
            .or_default()
            .open_record_keys
            .insert(record_key);
    }

    /// Note several records as open in one lock acquisition.
    pub fn mark_open_records(&self, community_id: &str, record_keys: &[String]) {
        let mut guard = self.inner.write();
        let entry = guard.entry(community_id.to_string()).or_default();
        for key in record_keys {
            entry.open_record_keys.insert(key.clone());
        }
    }

    /// Governance overflow chain for the community, in order.
    pub fn overflow_keys(&self, community_id: &str) -> Vec<String> {
        self.inner
            .read()
            .get(community_id)
            .map(|c| c.overflow_keys.clone())
            .unwrap_or_default()
    }

    /// Append to the overflow chain, skipping keys already present so a
    /// repeated discovery pass does not grow the chain.
    pub fn register_overflow_keys(&self, community_id: &str, keys: &[String]) {
        let mut guard = self.inner.write();
        let entry = guard.entry(community_id.to_string()).or_default();
        for key in keys {
            if !entry.overflow_keys.contains(key) {
                entry.overflow_keys.push(key.clone());
            }
        }
    }

    /// Drop all runtime state for a community — on leave, or on logout.
    /// Replace the observed roster for a community.
    ///
    /// Wholesale rather than merged: the poll re-derives the full set
    /// each tick, so a member who vanished from the registry must
    /// vanish here too. A merge would let a departed member linger
    /// forever, which is the bug the reclamation work exists to avoid
    /// on the slot side.
    pub fn set_members(&self, community_id: &str, members: HashMap<String, MemberRecord>) {
        self.inner
            .write()
            .entry(community_id.to_string())
            .or_default()
            .members = members;
    }

    /// Snapshot the observed roster.
    pub fn members(&self, community_id: &str) -> HashMap<String, MemberRecord> {
        self.inner
            .read()
            .get(community_id)
            .map(|c| c.members.clone())
            .unwrap_or_default()
    }

    /// How many members the last poll observed.
    pub fn member_count(&self, community_id: &str) -> usize {
        self.inner
            .read()
            .get(community_id)
            .map_or(0, |c| c.members.len())
    }

    /// Replace the merged per-member role map.
    pub fn set_member_roles(&self, community_id: &str, roles: HashMap<String, Vec<u32>>) {
        self.inner
            .write()
            .entry(community_id.to_string())
            .or_default()
            .member_roles = roles;
    }

    /// Snapshot the merged per-member role map.
    pub fn member_roles(&self, community_id: &str) -> HashMap<String, Vec<u32>> {
        self.inner
            .read()
            .get(community_id)
            .map(|c| c.member_roles.clone())
            .unwrap_or_default()
    }

    /// Record that a channel finished its initial history catch-up.
    pub fn mark_channel_synced(&self, community_id: &str, channel_id: &str) {
        self.inner
            .write()
            .entry(community_id.to_string())
            .or_default()
            .synced_channels
            .insert(channel_id.to_string());
    }

    pub fn remove(&self, community_id: &str) {
        self.inner.write().remove(community_id);
    }

    /// Drop everything. Called when the session is locked.
    pub fn clear(&self) {
        self.inner.write().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn governance_state_round_trips_and_is_absent_until_set() {
        let map = CommunityRuntimeMap::new();
        assert!(map.governance_state("c1").is_none());
        map.set_governance_state("c1", GovernanceState::default());
        assert!(map.governance_state("c1").is_some());
        // Unrelated community stays empty.
        assert!(map.governance_state("c2").is_none());
    }

    #[test]
    fn open_records_deduplicate() {
        let map = CommunityRuntimeMap::new();
        map.mark_open_record("c1", "rec-a".into());
        map.mark_open_record("c1", "rec-a".into());
        map.mark_open_records("c1", &["rec-b".into(), "rec-a".into()]);
        assert_eq!(map.open_record_keys("c1"), vec!["rec-a", "rec-b"]);
    }

    #[test]
    fn overflow_chain_keeps_order_and_skips_repeats() {
        let map = CommunityRuntimeMap::new();
        map.register_overflow_keys("c1", &["page-1".into(), "page-2".into()]);
        map.register_overflow_keys("c1", &["page-2".into(), "page-3".into()]);
        // Order matters — it is a chain, not a set.
        assert_eq!(map.overflow_keys("c1"), vec!["page-1", "page-2", "page-3"]);
    }

    #[test]
    fn remove_and_clear_drop_state() {
        let map = CommunityRuntimeMap::new();
        map.set_governance_state("c1", GovernanceState::default());
        map.set_governance_state("c2", GovernanceState::default());
        map.remove("c1");
        assert!(map.governance_state("c1").is_none());
        assert!(map.governance_state("c2").is_some());
        map.clear();
        assert!(map.governance_state("c2").is_none());
    }
}
