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

use std::collections::HashMap;

use rekindle_types::cross_device_sync::{
    DeviceList, DeviceListEntry, ReadState, ReadStateEntry, SyncCommunityRef, SyncManifest,
    SyncPreferences,
};

#[must_use]
pub fn merge_read_state(local: ReadState, remote: ReadState) -> ReadState {
    let mut by_key: HashMap<(String, String), ReadStateEntry> = HashMap::new();
    for entry in local.entries.into_iter().chain(remote.entries) {
        let key = (entry.community_id.clone(), entry.channel_id.clone());
        by_key
            .entry(key)
            .and_modify(|e| {
                if entry.last_read_lamport > e.last_read_lamport {
                    e.last_read_lamport = entry.last_read_lamport;
                }
            })
            .or_insert(entry);
    }
    let mut entries: Vec<ReadStateEntry> = by_key.into_values().collect();
    entries.sort_by(|a, b| {
        a.community_id
            .cmp(&b.community_id)
            .then(a.channel_id.cmp(&b.channel_id))
    });

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
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn entry(community: &str, channel: &str, lamport: u64) -> ReadStateEntry {
        ReadStateEntry {
            community_id: community.to_string(),
            channel_id: channel.to_string(),
            last_read_lamport: lamport,
        }
    }

    #[test]
    fn read_state_merge_takes_max_lamport_per_channel() {
        let local = ReadState {
            entries: vec![entry("c1", "ch1", 5), entry("c1", "ch2", 3)],
            onboarding_complete: HashMap::new(),
        };
        let remote = ReadState {
            entries: vec![entry("c1", "ch1", 10), entry("c2", "ch1", 1)],
            onboarding_complete: HashMap::new(),
        };
        let merged = merge_read_state(local, remote);
        assert_eq!(merged.entries.len(), 3);
        let ch1 = merged
            .entries
            .iter()
            .find(|e| e.community_id == "c1" && e.channel_id == "ch1")
            .unwrap();
        assert_eq!(ch1.last_read_lamport, 10, "max wins");
    }

    #[test]
    fn read_state_merge_unions_onboarding_complete() {
        let mut local_map = HashMap::new();
        local_map.insert("c1".to_string(), true);
        local_map.insert("c2".to_string(), false);
        let local = ReadState {
            entries: Vec::new(),
            onboarding_complete: local_map,
        };
        let mut remote_map = HashMap::new();
        remote_map.insert("c2".to_string(), true);
        remote_map.insert("c3".to_string(), true);
        let remote = ReadState {
            entries: Vec::new(),
            onboarding_complete: remote_map,
        };
        let merged = merge_read_state(local, remote);
        // c1 stays true (local-only); c2 flips to true (OR with remote);
        // c3 added from remote.
        assert_eq!(merged.onboarding_complete.get("c1"), Some(&true));
        assert_eq!(merged.onboarding_complete.get("c2"), Some(&true));
        assert_eq!(merged.onboarding_complete.get("c3"), Some(&true));
    }

    #[test]
    fn preferences_merge_takes_latest_lamport() {
        let local = SyncPreferences {
            theme: Some("light".to_string()),
            lamport: 5,
            ..Default::default()
        };
        let remote = SyncPreferences {
            theme: Some("dark".to_string()),
            lamport: 6,
            ..Default::default()
        };
        let merged = merge_preferences(local, remote);
        assert_eq!(merged.theme.as_deref(), Some("dark"));
    }

    #[test]
    fn preferences_merge_local_wins_on_higher_lamport() {
        let local = SyncPreferences {
            theme: Some("light".to_string()),
            lamport: 9,
            ..Default::default()
        };
        let remote = SyncPreferences {
            theme: Some("dark".to_string()),
            lamport: 6,
            ..Default::default()
        };
        let merged = merge_preferences(local, remote);
        assert_eq!(merged.theme.as_deref(), Some("light"));
    }

    #[test]
    fn manifest_merge_unions_and_takes_latest_joined_at() {
        let local = SyncManifest {
            communities: vec![SyncCommunityRef {
                community_id: "c1".to_string(),
                joined_at: 100,
                display_name: "old".to_string(),
            }],
            lamport: 1,
        };
        let remote = SyncManifest {
            communities: vec![
                SyncCommunityRef {
                    community_id: "c1".to_string(),
                    joined_at: 200, // newer
                    display_name: "renamed".to_string(),
                },
                SyncCommunityRef {
                    community_id: "c2".to_string(),
                    joined_at: 50,
                    display_name: "added".to_string(),
                },
            ],
            lamport: 5,
        };
        let merged = merge_manifest(local, remote);
        assert_eq!(merged.communities.len(), 2);
        let c1 = &merged.communities[0];
        assert_eq!(c1.community_id, "c1");
        assert_eq!(c1.joined_at, 200, "later joined_at wins");
        assert_eq!(c1.display_name, "renamed");
        assert_eq!(merged.lamport, 5, "max lamport");
    }

    #[test]
    fn device_list_merge_propagates_unpair() {
        let local = DeviceList {
            devices: vec![DeviceListEntry {
                device_id: "d1".to_string(),
                device_public_key: "pk1".to_string(),
                display_name: "Laptop".to_string(),
                paired_at: 100,
                unpaired_at: None,
            }],
            lamport: 1,
        };
        let remote = DeviceList {
            devices: vec![DeviceListEntry {
                device_id: "d1".to_string(),
                device_public_key: "pk1".to_string(),
                display_name: "Laptop".to_string(),
                paired_at: 100,
                unpaired_at: Some(200),
            }],
            lamport: 2,
        };
        let merged = merge_device_list(local, remote);
        assert_eq!(merged.devices[0].unpaired_at, Some(200));
    }

    #[test]
    fn device_list_merge_cannot_un_unpair() {
        // Remote (paired) doesn't overwrite local (unpaired). Once
        // unpaired, the device stays unpaired until re-paired.
        let local = DeviceList {
            devices: vec![DeviceListEntry {
                device_id: "d1".to_string(),
                device_public_key: "pk1".to_string(),
                display_name: "Laptop".to_string(),
                paired_at: 100,
                unpaired_at: Some(150),
            }],
            lamport: 2,
        };
        let remote = DeviceList {
            devices: vec![DeviceListEntry {
                device_id: "d1".to_string(),
                device_public_key: "pk1".to_string(),
                display_name: "Laptop renamed".to_string(),
                paired_at: 200, // newer paired_at
                unpaired_at: None,
            }],
            lamport: 3,
        };
        let merged = merge_device_list(local, remote);
        assert_eq!(merged.devices[0].unpaired_at, Some(150), "unpair sticks");
    }
}
