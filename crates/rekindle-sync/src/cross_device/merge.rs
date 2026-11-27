//! Phase 22.e-REDO — pure CRDT merge rules for the personal sync
//! record.
//!
//! Architecture §28.4 line 3074 prescribes the conflict-resolution
//! contract:
//!
//! - **Read state**: max Lamport per `(community_id, channel_id)`.
//!   Reading is monotonic — the maximum stays the maximum.
//! - **Preferences**: latest-Lamport-wins per record (LWW).
//! - **Manifest**: union of communities, latest `joined_at` wins
//!   per `community_id`.
//! - **Device list**: union; if the same `device_id` shows up with
//!   both `unpaired_at = None` and `unpaired_at = Some(_)`, the
//!   unpaired wins (cannot un-unpair without explicit re-pair).
//!
//! Lifted from `src-tauri/services/cross_device_sync/merge.rs`
//! verbatim — these were already pure functions; this is the
//! mechanical port under Invariant 7 (every CRDT decision lives in
//! the crate).

use std::collections::{BTreeMap, HashMap};

use rekindle_types::cross_device_sync::{
    DeviceList, DeviceListEntry, ReadScope, ReadState, ReadStateEntry, SyncCommunityRef,
    SyncManifest, SyncPreferences,
};

#[must_use]
pub fn merge_read_state(local: ReadState, remote: ReadState) -> ReadState {
    let mut by_scope: BTreeMap<ReadScope, u64> = BTreeMap::new();
    for entry in local.entries.into_iter().chain(remote.entries) {
        let seq = by_scope.entry(entry.scope).or_insert(0);
        *seq = (*seq).max(entry.last_read_seq);
    }
    let entries: Vec<ReadStateEntry> = by_scope
        .into_iter()
        .map(|(scope, last_read_seq)| ReadStateEntry {
            scope,
            last_read_seq,
        })
        .collect();

    // Architecture §28.4 — `onboarding_complete` is monotonic OR.
    // Once any paired device flips a community's flag to `true`,
    // every device must see it as `true`; clearing requires leaving
    // the community (which removes the entry locally on every device).
    let mut onboarding_complete = local.onboarding_complete;
    for (community_id, completed) in remote.onboarding_complete {
        let entry = onboarding_complete.entry(community_id).or_insert(false);
        *entry = *entry || completed;
    }

    ReadState {
        entries,
        onboarding_complete,
    }
}

#[must_use]
pub fn merge_preferences(local: SyncPreferences, remote: SyncPreferences) -> SyncPreferences {
    if remote.lamport > local.lamport {
        remote
    } else {
        local
    }
}

#[must_use]
pub fn merge_manifest(local: SyncManifest, remote: SyncManifest) -> SyncManifest {
    let mut by_id: HashMap<String, SyncCommunityRef> = HashMap::new();
    for entry in local.communities.into_iter().chain(remote.communities) {
        by_id
            .entry(entry.community_id.clone())
            .and_modify(|e| {
                if entry.joined_at > e.joined_at {
                    *e = entry.clone();
                }
            })
            .or_insert(entry);
    }
    let mut communities: Vec<SyncCommunityRef> = by_id.into_values().collect();
    communities.sort_by(|a, b| a.community_id.cmp(&b.community_id));
    SyncManifest {
        communities,
        lamport: local.lamport.max(remote.lamport),
    }
}

#[must_use]
pub fn merge_device_list(local: DeviceList, remote: DeviceList) -> DeviceList {
    let mut by_id: HashMap<String, DeviceListEntry> = HashMap::new();
    for entry in local.devices.into_iter().chain(remote.devices) {
        by_id
            .entry(entry.device_id.clone())
            .and_modify(|e| {
                // Unpaired wins — see module doc.
                if entry.unpaired_at.is_some() && e.unpaired_at.is_none() {
                    *e = entry.clone();
                } else if entry.paired_at > e.paired_at && entry.unpaired_at.is_none() {
                    e.display_name.clone_from(&entry.display_name);
                    e.paired_at = entry.paired_at;
                }
            })
            .or_insert(entry);
    }
    let mut devices: Vec<DeviceListEntry> = by_id.into_values().collect();
    devices.sort_by(|a, b| a.device_id.cmp(&b.device_id));
    DeviceList {
        devices,
        lamport: local.lamport.max(remote.lamport),
    }
}

#[cfg(test)]
#[path = "merge_tests.rs"]
mod tests;
