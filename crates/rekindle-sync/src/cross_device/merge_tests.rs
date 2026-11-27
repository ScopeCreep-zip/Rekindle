use super::*;
use std::collections::HashMap;

fn entry(community: &str, channel: &str, lamport: u64) -> ReadStateEntry {
    ReadStateEntry {
        scope: ReadScope::Channel {
            community_id: community.to_string(),
            channel_id: channel.to_string(),
        },
        last_read_seq: lamport,
    }
}

#[test]
fn read_state_merge_takes_max_lamport_per_channel() {
    let local = ReadState {
        entries: vec![entry("c1", "ch1", 5), entry("c1", "ch2", 3)],
        onboarding_complete: HashMap::new(),
    };
    let remote = ReadState {
        entries: vec![
            entry("c1", "ch1", 10),
            entry("c2", "ch1", 1),
            ReadStateEntry {
                scope: ReadScope::Dm {
                    peer_key: "p".to_string(),
                },
                last_read_seq: 4,
            },
        ],
        onboarding_complete: HashMap::new(),
    };
    let merged = merge_read_state(local, remote);
    assert_eq!(merged.entries.len(), 4);
    assert_eq!(merged.entries[0], entry("c1", "ch1", 10), "max wins");
    assert_eq!(
        merged.entries[3].scope,
        ReadScope::Dm {
            peer_key: "p".to_string()
        },
        "scopes in key order, DMs after channels"
    );
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
