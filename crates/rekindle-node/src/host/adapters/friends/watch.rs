//! The Host's friend watches on both hosts (plan E2.5, D23): every
//! accepted friend's profile record and the participant record their
//! chain to us is on, opened at unlock and again when the network comes
//! back. Nothing polls them: a watch change is read here (every changed
//! subkey, the first from the change itself) and applied, and a missed
//! change is recovered by the network-ready trigger, which re-opens the
//! watches and catches every chain up (`dm::stream::catch_up_all`).
//! A watch that fails to open (a record created moments ago is not
//! reachable yet) is kept in the friend stores' `watch_retries` and
//! re-opened on the maintenance tick with backoff; once it opens, what
//! changed meanwhile is read (plan E2.6).

use std::sync::atomic::Ordering;
use std::sync::Arc;

use rekindle_presence::FriendPresenceDeps;
use rekindle_records::lease::LeaseId;
use rekindle_transport::operations::friend::watch_retry::WatchTarget;
use rekindle_transport::operations::friend::FriendHost;
use rekindle_transport::operations::{dm, group_dm};
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
        for lease in held {
            self.apply_profile(lease).await;
        }
        dm::stream::catch_up_all(self).await;
        group_dm::receive::catch_up_all(self).await;
    }

    /// Apply what changed on the profile record `lease` holds while no
    /// watch of ours fired for it.
    async fn apply_profile(&self, lease: LeaseId) {
        let pool = self.node().ok().and_then(|node| node.records());
        let Some(profile) = pool.as_ref().and_then(|pool| pool.key_of(lease)) else {
            return;
        };
        let subkeys = rekindle_presence::FRIEND_WATCH_SUBKEYS;
        for (subkey, value) in read_changed(self, Some(lease), subkeys, None).await {
            rekindle_presence::handle_value_change(self, &profile, &[subkey], &value);
        }
    }

    /// The maintenance tick: re-open every failed watch whose retry is due
    /// and still wanted; one that opens has its changes read (its profile
    /// applied, its chain diffed), one that fails waits a longer backoff.
    pub(super) async fn retry_failed_watches(&self) {
        let now = self.ctx().clock.now_ms();
        let due = self.ctx().friend_stores.watch_retries.lock().due(now);
        if due.is_empty() {
            return;
        }
        let participants = participant_records(self).await;
        for (record, target) in due {
            let wanted = match &target {
                WatchTarget::Profile { peer } => self.wants_profile(peer, &record),
                WatchTarget::Participant { key } => match group_dm::receive::parse_watch_key(key) {
                    Some(_) => group_dm::receive::holds_chain(self, key, &record).await,
                    None => participants.iter().any(|(p, r)| p == key && *r == record),
                },
            };
            if !wanted {
                self.ctx()
                    .friend_stores
                    .watch_retries
                    .lock()
                    .settled(&record);
                continue;
            }
            match &target {
                WatchTarget::Profile { peer } => self.watch_friend(peer, &record).await,
                WatchTarget::Participant { key } => watch_participant(self, key, &record).await,
            }
            if self
                .ctx()
                .friend_stores
                .watch_retries
                .lock()
                .is_pending(&record)
            {
                continue;
            }
            tracing::debug!(record = %record, "a failed watch opened on retry");
            match &target {
                WatchTarget::Profile { peer } => {
                    let lease = self.ctx().friend_watches.leases.lock().get(peer).copied();
                    if let Some(lease) = lease {
                        self.apply_profile(lease).await;
                    }
                }
                WatchTarget::Participant { key } => diff_participant(self, key, &record).await,
            }
        }
    }

    /// Whether `peer` is an accepted friend whose profile is `record`.
    fn wants_profile(&self, peer: &str, record: &str) -> bool {
        self.ctx().session.read().as_ref().is_some_and(|s| {
            s.friends.get(peer).is_some_and(|f| {
                f.friendship_state == FriendshipState::Accepted
                    && f.dht_record_key.as_deref() == Some(record)
            })
        })
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
            diff_participant(self, &peer, record).await;
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
        group_dm::receive::watch_all(self).await;
    }
}

/// Diff the chain on the participant record `record` held under `key` (a
/// friend, or a member's group chain, `group_dm::receive::watch_key`).
async fn diff_participant(host: &HostFriends, key: &str, record: &str) {
    if group_dm::receive::parse_watch_key(key).is_some() {
        group_dm::receive::on_watch_change(host, key, record).await;
    } else {
        dm::stream::on_watch_change(host, key, record).await;
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
/// lease (the one held for them before is released). A failed open or
/// watch is retried on the maintenance tick (`retry_failed_watches`).
pub(super) async fn watch_participant(host: &HostFriends, peer: &str, record: &str) {
    let Some(pool) = host.node().ok().and_then(|node| node.records()) else {
        return;
    };
    let failed = |e: &dyn std::fmt::Display, what: &str| {
        tracing::warn!(peer = %peer, error = %e, "participant record not {what}; retried");
        let target = WatchTarget::Participant {
            key: peer.to_string(),
        };
        let now = host.ctx().clock.now_ms();
        let rng = host.ctx().rng.as_ref();
        let mut retries = host.ctx().friend_stores.watch_retries.lock();
        retries.failed(record, target, now, rng);
    };
    let lease = match pool.acquire(record, None).await {
        Ok(lease) => lease,
        Err(e) => return failed(&e, "opened"),
    };
    match pool.watch_all(lease).await {
        Ok(()) => host
            .ctx()
            .friend_stores
            .watch_retries
            .lock()
            .settled(record),
        Err(e) => failed(&e, "watched"),
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
