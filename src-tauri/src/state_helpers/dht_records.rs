//! DHT record-key storage and tracking on the node / manager handles.

use std::sync::Arc;

use crate::state::AppState;

/// Which type of DHT record is being stored on the node/manager handles.
///
/// `Profile` and `FriendList` carry an optional owner keypair (set on creation,
/// `None` on reopen). Account and Mailbox never carry a keypair.
pub enum DhtRecordType {
    Profile(Option<veilid_core::KeyPair>),
    FriendList(Option<veilid_core::KeyPair>),
    Account,
    Mailbox,
}

/// Store a DHT record key on `NodeHandle` and track it in `DHTManagerHandle`.
///
/// Acquires `node.write()` then `dht_manager.write()` sequentially (matching
/// the lock ordering used everywhere else). Each guard is dropped before the
/// next is acquired — safe with `parking_lot`'s `!Send` guards.
pub fn store_dht_record(state: &Arc<AppState>, key: &str, record_type: &DhtRecordType) {
    {
        let mut node = state.node.write();
        if let Some(ref mut nh) = *node {
            match &record_type {
                DhtRecordType::Profile(kp) => nh.set_profile_dht(key.to_string(), kp.clone()),
                DhtRecordType::FriendList(kp) => {
                    nh.set_friend_list_dht(key.to_string(), kp.clone());
                }
                DhtRecordType::Account => nh.set_account_dht(key.to_string()),
                DhtRecordType::Mailbox => nh.set_mailbox_dht(key.to_string()),
            }
        }
    }
    {
        let mut dht_mgr = state.dht_manager.write();
        if let Some(ref mut mgr) = *dht_mgr {
            match &record_type {
                DhtRecordType::Profile(_) => mgr.set_profile_key(key),
                DhtRecordType::FriendList(_) => mgr.set_friend_list_key(key),
                DhtRecordType::Account | DhtRecordType::Mailbox => {
                    mgr.track_open_record(key.to_string());
                }
            }
        }
    }
}

/// Track multiple DHT record keys as opened in this session.
///
/// Acquires `dht_manager.write()` once and inserts all keys. Useful for
/// compound records (account children, conversation children) where several
/// sub-records are created together.
pub fn track_open_records(state: &Arc<AppState>, keys: &[String]) {
    let mut dht_mgr = state.dht_manager.write();
    if let Some(ref mut mgr) = *dht_mgr {
        for k in keys {
            mgr.track_open_record(k.clone());
        }
    }
}

/// Remove multiple DHT record keys from the global tracking set.
pub fn untrack_records(state: &Arc<AppState>, keys: &[String]) {
    let mut dht_mgr = state.dht_manager.write();
    if let Some(ref mut mgr) = *dht_mgr {
        for k in keys {
            mgr.untrack_record(k);
        }
    }
}

/// Collect all opened DHT record keys for a community from its `CommunityRecords`.
///
/// Returns the keys and marks the community's records as closed in state.
pub fn collect_and_clear_community_records(
    state: &Arc<AppState>,
    community_id: &str,
) -> Vec<String> {
    let mut communities = state.communities.write();
    let Some(cs) = communities.get_mut(community_id) else {
        return Vec::new();
    };
    let records = &mut cs.open_community_records;
    let mut keys = Vec::new();
    if let Some(ref k) = records.governance_key {
        keys.push(k.clone());
    }
    if let Some(ref k) = records.registry_key {
        keys.push(k.clone());
    }
    keys.append(&mut records.channel_keys);
    records.governance_key = None;
    records.registry_key = None;
    records.registry_writer = None;
    records.records_open = false;
    keys
}
