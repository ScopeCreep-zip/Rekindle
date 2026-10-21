//! The Host's watches on our open invites' request inboxes (plan E2.5,
//! D17): a lease per inbox, watched on the one-time writer's subkey, held
//! while the invite is `pending` and re-watched at unlock. A change is
//! opened as any inbound 1:1 frame (`operations::friend::inbox::received`).

use std::sync::Arc;

use rekindle_records::lease::LeaseId;
use rekindle_records::port::RecordPort;
use rekindle_transport::operations::friend::{inbox, FriendHost};
use rekindle_types::dht_layout::request_inbox;

use super::HostFriends;

/// Hold `lease` on the inbox `record_key` and watch its writer's subkey.
pub(super) async fn hold(
    host: &HostFriends,
    pool: &Arc<dyn RecordPort>,
    record_key: &str,
    lease: LeaseId,
) {
    if let Err(e) = pool.watch(lease, &[request_inbox::REQUEST]).await {
        tracing::warn!(error = %e, "invite inbox not watched");
    }
    let previous = host
        .ctx()
        .friend_watches
        .inboxes
        .lock()
        .insert(record_key.to_string(), lease);
    if let Some(previous) = previous.filter(|p| *p != lease) {
        pool.release(previous).await;
    }
}

/// Stop watching the inbox `record_key`: releasing its lease closes the
/// record, which ends its watch.
pub(super) async fn release(host: &HostFriends, record_key: &str) {
    let lease = host.ctx().friend_watches.inboxes.lock().remove(record_key);
    let pool = host.node().ok().and_then(|node| node.records());
    if let (Some(lease), Some(pool)) = (lease, pool) {
        pool.release(lease).await;
    }
}

/// Watch every pending invite's inbox again (an unlock).
pub(super) async fn rewatch_pending(host: &HostFriends) {
    let (Ok(owner), Ok(db)) = (host.ctx().require_owner(), host.ctx().db.current()) else {
        return;
    };
    let keys = db
        .call(move |conn| rekindle_db::repo::outgoing_invites::pending_inboxes(conn, &owner))
        .await;
    let (Ok(keys), Some(pool)) = (keys, host.node().ok().and_then(|node| node.records())) else {
        return;
    };
    for key in keys {
        match pool.acquire(&key, None).await {
            Ok(lease) => hold(host, &pool, &key, lease).await,
            Err(e) => tracing::warn!(error = %e, "invite inbox not reopened"),
        }
    }
}

impl HostFriends {
    /// A watched record changed: when it is one of our invites' inboxes,
    /// open what its writer wrote; whether it was. `value` is the inline
    /// value of the first changed subkey.
    pub fn request_inbox_change(
        &self,
        record_key: &str,
        subkeys: &[u32],
        value: Option<&[u8]>,
    ) -> bool {
        let Some(lease) = self
            .ctx()
            .friend_watches
            .inboxes
            .lock()
            .get(record_key)
            .copied()
        else {
            return false;
        };
        if !subkeys.contains(&request_inbox::REQUEST) {
            return true;
        }
        let inline = (subkeys.first() == Some(&request_inbox::REQUEST))
            .then(|| value.map(<[u8]>::to_vec))
            .flatten();
        let host = self.clone();
        self.scope()
            .spawn_or_drop("open invite inbox request", async move {
                let value = match inline {
                    Some(value) => Some(value),
                    None => read(&host, lease).await,
                };
                if let Some(value) = value {
                    inbox::received(&host, &value).await;
                }
            });
        true
    }
}

async fn read(host: &HostFriends, lease: LeaseId) -> Option<Vec<u8>> {
    let pool = host.node().ok()?.records()?;
    match pool.get(lease, request_inbox::REQUEST, true).await {
        Ok(value) => value.map(|v| v.data),
        Err(e) => {
            tracing::debug!(error = %e, "invite inbox request not read");
            None
        }
    }
}
