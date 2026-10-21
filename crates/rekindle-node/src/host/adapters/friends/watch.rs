//! The Host's friend watches on both hosts (plan E2.5, D23): every
//! accepted friend's profile record and the participant record their
//! chain to us is on, opened at unlock and again when the network comes
//! back. Nothing polls them: a watch change is read here (every changed
//! subkey, the first from the change itself) and applied, and a missed
//! change is recovered by the network-ready trigger, which re-opens the
//! watches and catches every chain up (`dm::stream::catch_up_all`).

use std::sync::atomic::Ordering;
use std::sync::Arc;

use rekindle_presence::FriendPresenceDeps;
use rekindle_records::lease::LeaseId;
use rekindle_transport::operations::dm;
use rekindle_transport::operations::friend::FriendHost;
use rekindle_types::friend::FriendshipState;

use super::{inbox_watch, HostFriends};
use crate::daemon::dispatch::DaemonContext;

/// Open every accepted friend's watches and every pending invite's inbox
/// watch, in `scope` (the unlock's).
pub(crate) fn watch_friends(ctx: &DaemonContext, scope: &Arc<rekindle_lifecycle::SessionScope>) {
    let host = HostFriends::of(ctx);
    scope.spawn_or_drop("friend watches", async move {
        host.rewatch().await;
        inbox_watch::rewatch_pending(&host).await;
    });
}

impl HostFriends {
    /// The network is (not) ready. Becoming ready while unlocked re-opens
    /// the friends' watches and catches every chain up: what changed while
    /// we were away fired no watch of ours.
    pub fn network_changed(&self, ready: bool) {
        let was = self
            .ctx()
            .friend_watches
            .network_ready
            .swap(ready, Ordering::SeqCst);
        if ready && !was && self.ctx().session.read().is_some() {
            let host = self.clone();
            self.scope()
                .spawn_or_drop("friend watch heal", async move { host.heal().await });
        }
    }

    /// Re-open every friend's watches, apply what changed on their profiles
    /// meanwhile (no watch of ours fired for it), then diff every chain.
    pub async fn heal(&self) {
        self.rewatch().await;
        let held: Vec<LeaseId> = self
            .ctx()
            .friend_watches
            .leases
            .lock()
            .values()
            .copied()
            .collect();
        let pool = self.node().ok().and_then(|node| node.records());
        for lease in held {
            let Some(profile) = pool.as_ref().and_then(|pool| pool.key_of(lease)) else {
                continue;
            };
            let subkeys = rekindle_presence::FRIEND_WATCH_SUBKEYS;
            for (subkey, value) in read_changed(self, Some(lease), subkeys, None).await {
                rekindle_presence::handle_value_change(self, &profile, &[subkey], &value);
            }
        }
        dm::stream::catch_up_all(self).await;
    }

    /// A watched record changed: an invite's inbox, a friend's participant
    /// record or a friend's profile is handled here; whether it was one.
    /// `inline` is the first changed subkey's value, carried by the change.
    pub async fn on_watched_change(
        &self,
        record: &str,
        subkeys: &[u32],
        inline: Option<&[u8]>,
    ) -> bool {
        if self.request_inbox_change(record, subkeys, inline) {
            return true;
        }
        if let Some(peer) = participant_peer(self, record) {
            dm::stream::on_watch_change(self, &peer, record).await;
            return true;
        }
        let Some(friend) = self.friend_for_dht_key(record) else {
            return false;
        };
        let lease = self
            .ctx()
            .friend_watches
            .leases
            .lock()
            .get(&friend)
            .copied();
        for (subkey, value) in read_changed(self, lease, subkeys, inline).await {
            rekindle_presence::handle_value_change(self, record, &[subkey], &value);
        }
        true
    }

    /// Watch every accepted friend's profile and participant record.
    async fn rewatch(&self) {
        let profiles: Vec<(String, String)> = self
            .ctx()
            .session
            .read()
            .as_ref()
            .map(|s| {
                s.friends
                    .values()
                    .filter(|f| f.friendship_state == FriendshipState::Accepted)
                    .filter_map(|f| f.dht_record_key.clone().map(|k| (f.public_key.clone(), k)))
                    .collect()
            })
            .unwrap_or_default();
        for (peer, profile) in profiles {
            self.watch_friend(&peer, &profile).await;
        }
        for (peer, record) in participant_records(self).await {
            watch_participant(self, &peer, &record).await;
        }
    }
}

/// Every changed subkey's value: the first from the change, the rest read
/// through the friend's lease.
async fn read_changed(
    host: &HostFriends,
    lease: Option<LeaseId>,
    subkeys: &[u32],
    inline: Option<&[u8]>,
) -> Vec<(u32, Vec<u8>)> {
    let mut values = Vec::with_capacity(subkeys.len());
    let mut rest = subkeys;
    if let (Some((first, tail)), Some(value)) = (subkeys.split_first(), inline) {
        values.push((*first, value.to_vec()));
        rest = tail;
    }
    let pool = host.node().ok().and_then(|node| node.records());
    if let (false, Some(lease), Some(pool)) = (rest.is_empty(), lease, pool) {
        match pool.read_changed(lease, rest).await {
            Ok(read) => values.extend(read.into_iter().map(|(s, v)| (s, v.data))),
            Err(e) => tracing::debug!(error = %e, "changed friend subkeys not read"),
        }
    }
    values.retain(|(_, value)| !value.is_empty());
    values
}

/// Each accepted friend's participant record we read their chain from.
async fn participant_records(host: &HostFriends) -> Vec<(String, String)> {
    let (Ok(owner), Ok(db)) = (host.ctx().require_owner(), host.ctx().db.current()) else {
        return Vec::new();
    };
    db.call(move |conn| rekindle_db::repo::friends::peer_participant_record_key(conn, &owner, None))
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "participant records not read");
            Vec::new()
        })
}

/// Watch every subkey of `peer`'s participant record `record`, holding its
/// lease (the one held for them before is released).
pub(super) async fn watch_participant(host: &HostFriends, peer: &str, record: &str) {
    let Some(pool) = host.node().ok().and_then(|node| node.records()) else {
        return;
    };
    let lease = match pool.acquire(record, None).await {
        Ok(lease) => lease,
        Err(e) => return tracing::warn!(peer = %peer, error = %e, "participant record not opened"),
    };
    if let Err(e) = pool.watch_all(lease).await {
        tracing::warn!(peer = %peer, error = %e, "participant record not watched");
    }
    let previous = host
        .ctx()
        .friend_watches
        .participants
        .lock()
        .insert(peer.to_string(), (record.to_string(), lease));
    if let Some((_, previous)) = previous.filter(|(_, p)| *p != lease) {
        pool.release(previous).await;
    }
}

/// Stop watching `peer`'s participant record.
pub(super) async fn release_participant(host: &HostFriends, peer: &str) {
    let held = host.ctx().friend_watches.participants.lock().remove(peer);
    let pool = host.node().ok().and_then(|node| node.records());
    if let (Some((_, lease)), Some(pool)) = (held, pool) {
        pool.release(lease).await;
    }
}

fn participant_peer(host: &HostFriends, record: &str) -> Option<String> {
    host.ctx()
        .friend_watches
        .participants
        .lock()
        .iter()
        .find(|(_, (key, _))| key == record)
        .map(|(peer, _)| peer.clone())
}
