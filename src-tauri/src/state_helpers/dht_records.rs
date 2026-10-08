//! DHT record-key storage and tracking on the node / manager handles.

use std::sync::Arc;

use crate::state::AppState;

/// Which type of DHT record is being stored on the node/manager handles.
///
/// `Profile` and `FriendList` carry their owner keypair (and the profile its
/// session lease). Account and Mailbox never carry a keypair.
pub enum DhtRecordType {
    Profile(veilid_core::KeyPair, rekindle_records::lease::LeaseId),
    FriendList(veilid_core::KeyPair),
    Account,
    Mailbox,
}

/// Store one of our own record keys on `NodeHandle`.
///
/// These four records are held by the session's record pool, which ends
/// them at logout (plan C7.4).
pub fn store_dht_record(state: &Arc<AppState>, key: &str, record_type: &DhtRecordType) {
    let mut node = state.node.write();
    if let Some(ref mut nh) = *node {
        match &record_type {
            DhtRecordType::Profile(kp, lease) => {
                nh.set_profile_dht(key.to_string(), kp.clone(), *lease);
            }
            DhtRecordType::FriendList(kp) => {
                nh.set_friend_list_dht(key.to_string(), kp.clone());
            }
            DhtRecordType::Account => nh.set_account_dht(key.to_string()),
            DhtRecordType::Mailbox => nh.set_mailbox_dht(key.to_string()),
        }
    }
}

/// Release a friend's held profile lease (removal or block). Releasing
/// the last borrow closes the record, which is what cancels its watch;
/// unregistering the `dht_key_to_friend` mapping alone does not.
pub async fn release_friend_record(state: &Arc<AppState>, friend_key: &str) {
    let held = state.friend_leases.lock().remove(friend_key);
    if let (Some(lease), Ok(pool)) = (held, super::record_pool(state)) {
        pool.release(lease).await;
    }
}
