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

/// Close one DHT record and drop it from the tracking set.
///
/// Closing is what cancels the record's watch — veilid-core documents
/// `close_dht_record` as "the release half of the open/close pair;
/// cancels the record's watch (in the background)". Unregistering a
/// friend's `dht_key_to_friend` mapping does not do this: the record
/// stays open and the watch keeps delivering, so a removed or blocked
/// peer went on being observed for the life of the process.
pub async fn close_and_untrack(state: &Arc<AppState>, key: &str) {
    let rc = {
        let node = state.node.read();
        node.as_ref().map(|nh| nh.routing_context.clone())
    };
    if let (Some(rc), Ok(parsed)) = (rc, key.parse::<veilid_core::RecordKey>()) {
        if let Err(e) = rc.close_dht_record(parsed).await {
            tracing::debug!(key, error = %e, "close friend record");
        }
    } else {
        tracing::debug!(key, "friend record not closed — no node or bad key");
    }
    untrack_records(state, std::slice::from_ref(&key.to_string()));
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
    // §10 teardown lives on the inventory itself, so the overflow chain
    // (the author's spill pages and any followed chain) cannot be
    // forgotten here the way a hand-rolled sweep can forget it.
    cs.open_community_records.take_all_for_close()
}
