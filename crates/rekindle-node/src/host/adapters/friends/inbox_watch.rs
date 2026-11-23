//! The Host's watches on our open invites' request inboxes (plan E2.5,
//! D17): a lease per inbox, watched on the one-time writer's subkey, held
//! while the invite is `pending` and re-watched at unlock. A change is
//! opened as any inbound 1:1 frame (`operations::friend::inbox::received`).
//! Our open requests' reply inboxes (plan RV2) are held the same way in
//! their own map, re-held at unlock and released when the request settles
//! ([`rewatch_open_requests`]); what lands there is opened by
//! `inbox::received_reply`. Our friend code's request inbox (plan RV2e) is
//! held in a third map, reopened at unlock; what lands there is opened by
//! `inbox::received_reply` too, and the slot rolled (`code_inbox::landed`).

use rekindle_records::port::OpenIntent;
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
    hold_in(&host.ctx().friend_watches.inboxes, pool, record_key, lease).await;
}

/// Hold `lease` on our request's reply inbox `record_key` and watch its
/// writer's subkey (plan RV2).
pub(super) async fn hold_reply(
    host: &HostFriends,
    pool: &Arc<dyn RecordPort>,
    record_key: &str,
    lease: LeaseId,
) {
    hold_in(&host.ctx().friend_watches.replies, pool, record_key, lease).await;
}

/// Hold `lease` on our friend code's request inbox `record_key` and watch
/// its writer's subkey (plan RV2e).
pub(super) async fn hold_code(
    host: &HostFriends,
    pool: &Arc<dyn RecordPort>,
    record_key: &str,
    lease: LeaseId,
) {
    hold_in(&host.ctx().friend_watches.codes, pool, record_key, lease).await;
}

/// Stop watching our friend code's inbox `record_key` (rolled).
pub(super) async fn release_code(host: &HostFriends, record_key: &str) {
    let lease = host.ctx().friend_watches.codes.lock().remove(record_key);
    let pool = host.node().ok().and_then(|node| node.records());
    if let (Some(lease), Some(pool)) = (lease, pool) {
        pool.release(lease).await;
    }
}

async fn hold_in(
    map: &parking_lot::Mutex<std::collections::BTreeMap<String, LeaseId>>,
    pool: &Arc<dyn RecordPort>,
    record_key: &str,
    lease: LeaseId,
) {
    if let Err(e) = pool.watch(lease, &[request_inbox::REQUEST]).await {
        tracing::warn!(error = %e, "request inbox not watched");
    }
    let previous = map.lock().insert(record_key.to_string(), lease);
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
        match pool.acquire(&key, None, OpenIntent::Hold).await {
            Ok(lease) => hold(host, &pool, &key, lease).await,
            Err(e) => tracing::warn!(error = %e, "invite inbox not reopened"),
        }
    }
    let owner = host.ctx().require_owner();
    let code = match owner {
        Ok(owner) => db
            .call(move |conn| rekindle_db::repo::outgoing_invites::code_inbox_key(conn, &owner))
            .await
            .ok()
            .flatten(),
        Err(_) => None,
    };
    if let Some(key) = code {
        match pool.acquire(&key, None, OpenIntent::Hold).await {
            Ok(lease) => hold_code(host, &pool, &key, lease).await,
            Err(e) => tracing::warn!(error = %e, "friend code inbox not reopened"),
        }
    }
}

/// Hold and watch exactly our open requests' reply inboxes (plan RV2): one
/// not held is reopened (an unlock), and one whose request settled is
/// released and its owner keypair forgotten.
pub(super) async fn rewatch_open_requests(host: &HostFriends) {
    let (Ok(owner), Ok(db)) = (host.ctx().require_owner(), host.ctx().db.current()) else {
        return;
    };
    let keys = db
        .call(move |conn| rekindle_db::repo::friends::reply_inboxes(conn, &owner))
        .await;
    let (Ok(keys), Some(pool)) = (keys, host.node().ok().and_then(|node| node.records())) else {
        return;
    };
    let settled: Vec<(String, LeaseId)> = {
        let mut held = host.ctx().friend_watches.replies.lock();
        let gone: Vec<String> = held.keys().filter(|k| !keys.contains(k)).cloned().collect();
        gone.into_iter()
            .filter_map(|k| held.remove(&k).map(|lease| (k, lease)))
            .collect()
    };
    for (key, lease) in settled {
        pool.release(lease).await;
        host.forget_inbox_owner(&key);
    }
    for key in keys {
        if host.ctx().friend_watches.replies.lock().contains_key(&key) {
            continue;
        }
        match pool.acquire(&key, None, OpenIntent::Hold).await {
            Ok(lease) => hold_reply(host, &pool, &key, lease).await,
            Err(e) => tracing::warn!(error = %e, "reply inbox not reopened"),
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
        let invite = self
            .ctx()
            .friend_watches
            .inboxes
            .lock()
            .get(record_key)
            .copied();
        let reply = self
            .ctx()
            .friend_watches
            .replies
            .lock()
            .get(record_key)
            .copied();
        let code = self
            .ctx()
            .friend_watches
            .codes
            .lock()
            .get(record_key)
            .copied();
        // A code's request is a first contact's durable copy, as a reply is.
        let (lease, is_reply) = match (invite, reply.or(code)) {
            (Some(lease), _) => (lease, false),
            (None, Some(lease)) => (lease, true),
            (None, None) => return false,
        };
        let rolls = code.is_some().then(|| record_key.to_owned());
        if !subkeys.contains(&request_inbox::REQUEST) {
            return true;
        }
        let inline = (subkeys.first() == Some(&request_inbox::REQUEST))
            .then(|| value.map(<[u8]>::to_vec))
            .flatten();
        let host = self.clone();
        self.scope()
            .spawn_or_drop("open invite inbox request", async move {
                // The carried value first; the network read always runs, so
                // Veilid's report mark clears (`ValueChange`'s contract), and
                // a newer request it finds is handled too.
                let open = |value: Vec<u8>| {
                    let host = host.clone();
                    async move {
                        if is_reply {
                            inbox::received_reply(&host, &value).await;
                        } else {
                            inbox::received(&host, &value).await;
                        }
                    }
                };
                if let Some(value) = &inline {
                    open(value.clone()).await;
                }
                if let Some(value) = read(&host, lease).await {
                    if inline.as_ref() != Some(&value) {
                        open(value).await;
                    }
                }
                // The slot took its one write: the next redeemer's is a new one.
                if let Some(slot) = rolls {
                    rekindle_transport::operations::friend::code_inbox::landed(&host, &slot).await;
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
