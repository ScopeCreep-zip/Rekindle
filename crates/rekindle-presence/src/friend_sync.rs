//! Phase 22.c-REDO — friend-sync orchestrator.
//!
//! Pre-port lived in `src-tauri/services/sync_service.rs` as ~390
//! LoC of duplicated per-subkey parse/state/emit logic. Here it
//! parameterises over [`FriendPresenceDeps`] and reuses the
//! existing `handle_value_change` per-subkey processing so each
//! status/game/route fetch funnels through the SAME state-update +
//! emit path that the live DHT watch fires on.
//!
//! Architecture references:
//! - §13.4 presence cadence (force-poll fallback when watches fail)
//! - §26 W26 — friend profile record subkey layout (2=status,
//!   4=game, 6=route blob; 5, the prekey device list, is read only by a
//!   first contact, plan E2.4)

use std::collections::HashSet;
use std::sync::Arc;

use crate::deps::{FriendPresenceDeps, FriendPresenceEvent};
use crate::friend::{handle_value_change, STALE_PRESENCE_THRESHOLD_MS};

/// Profile subkeys we force-poll each tick (mirror of
/// [`crate::friend::FRIEND_WATCH_SUBKEYS`]).
const SYNC_POLL_SUBKEYS: &[u32] = &[2, 4, 6];

/// Run a single friend-sync tick: iterate every friend with a DHT
/// record, register the DHT key mapping, start a watch if one
/// hasn't fired this session, force-poll each profile subkey, and
/// finally mark stale-heartbeat friends offline.
///
/// `watched_keys` is the per-loop set of `dht_record_key`s where
/// the live watch has been established. The orchestrator clears
/// keys for friends in [`FriendPresenceDeps::unwatched_friends`]
/// so they get re-watched on the next tick.
///
/// `first_tick` + `force_all` drive when to force the DHT
/// `force_refresh=true` flag — the first tick or every 10th tick
/// per pre-port `sync_service::start_sync_loop` cadence. Stops before
/// the next friend's calls once `stop` is cancelled (plan C4.L1).
pub async fn sync_friends<D, S>(
    deps: Arc<D>,
    watched_keys: &mut HashSet<String, S>,
    first_tick: bool,
    force_all: bool,
    stop: &tokio_util::sync::CancellationToken,
) where
    D: FriendPresenceDeps,
    S: std::hash::BuildHasher,
{
    let friends_with_dht = deps.friends_with_dht_keys();
    let unwatched = deps.unwatched_friends();

    // Clear watched_keys for friends whose watches died so they
    // get re-watched this tick.
    for friend_key in &unwatched {
        // Re-derive the dht key for this friend by matching against
        // the friends_with_dht list (cheap — small N).
        if let Some((_, dht_key)) = friends_with_dht.iter().find(|(fk, _)| fk == friend_key) {
            watched_keys.remove(dht_key);
        }
    }

    for (friend_key, dht_key) in &friends_with_dht {
        if stop.is_cancelled() {
            return;
        }
        if !watched_keys.contains(dht_key) {
            // `watch_friend` returns `Ok(())` even when the underlying
            // watch couldn't be established (it adds the friend to the
            // unwatched set instead). Either way we mark this key as
            // attempted so we don't spam the watch call every tick.
            let _ = crate::watch_friend(Arc::clone(&deps), friend_key, dht_key).await;
            watched_keys.insert(dht_key.clone());
        }

        let force_refresh = first_tick || force_all || unwatched.contains(friend_key);
        force_poll_subkeys(deps.as_ref(), dht_key, force_refresh).await;
    }

    check_stale_friend_presences(deps.as_ref());
    tracing::debug!(friends = friends_with_dht.len(), "friend sync complete");
}

/// Force-poll each watched-presence subkey for one friend and
/// funnel the bytes through `handle_value_change` so the status /
/// game / route paths share the same state-update + emit code
/// that the live DHT watch fires on.
async fn force_poll_subkeys<D: FriendPresenceDeps>(deps: &D, dht_key: &str, force_refresh: bool) {
    for &subkey in SYNC_POLL_SUBKEYS {
        let Some(bytes) = deps
            .fetch_friend_dht_subkey(dht_key, subkey, force_refresh)
            .await
        else {
            continue;
        };
        handle_value_change(deps, dht_key, &[subkey], &bytes);
    }
}

/// Mark every friend with a stale `last_heartbeat_at` as offline
/// and emit `FriendOffline` (privacy-gated by `is_friend_accepted`).
/// Pre-port `check_stale_presences`.
pub fn check_stale_friend_presences<D: FriendPresenceDeps>(deps: &D) {
    let now = deps.now_ms();
    for friend_key in deps.find_stale_friend_heartbeats(STALE_PRESENCE_THRESHOLD_MS) {
        let is_accepted = deps.is_friend_accepted(&friend_key);
        deps.set_friend_offline(&friend_key, now);
        if is_accepted {
            tracing::info!(friend = %friend_key, "stale heartbeat — marking offline");
            deps.emit(FriendPresenceEvent::FriendOffline { friend_key });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use parking_lot::Mutex;
    use std::collections::HashMap;

    use super::*;
    use crate::deps::{GameInfoSnapshot, PresenceError, SetFriendStatusOutcome};
    use crate::status::UserStatusKind;

    #[derive(Default)]
    struct MockState {
        friends_with_dht: Vec<(String, String)>,
        unwatched: HashSet<String>,
        subkey_bytes: HashMap<(String, u32), Vec<u8>>,
        stale_friends: Vec<String>,
        // Recorders
        calls_friends_with_dht: u32,
        calls_unwatched: u32,
        calls_set_dht_key: Vec<(String, String)>,
        calls_open_friend: Vec<String>,
        calls_watch_subkeys: Vec<(u64, Vec<u32>)>,
        calls_fetch_subkey: Vec<(String, u32, bool)>,
        calls_set_offline: Vec<(String, i64)>,
        calls_is_accepted: Vec<String>,
        calls_emit: Vec<FriendPresenceEvent>,
        calls_set_unwatched: Vec<(String, bool)>,
        calls_hold: Vec<(String, u64)>,
        calls_find_stale: Vec<i64>,
    }

    struct MockDeps {
        state: Mutex<MockState>,
        accepted: HashSet<String>,
    }

    impl Default for MockDeps {
        fn default() -> Self {
            Self {
                state: Mutex::new(MockState::default()),
                accepted: HashSet::new(),
            }
        }
    }

    #[async_trait]
    impl FriendPresenceDeps for MockDeps {
        fn now_ms(&self) -> i64 {
            1_000_000
        }
        fn friend_for_dht_key(&self, dht_key: &str) -> Option<String> {
            self.state
                .lock()
                .friends_with_dht
                .iter()
                .find(|(_, dk)| dk == dht_key)
                .map(|(fk, _)| fk.clone())
        }
        fn is_friend_accepted(&self, friend_key: &str) -> bool {
            self.state
                .lock()
                .calls_is_accepted
                .push(friend_key.to_string());
            self.accepted.contains(friend_key)
        }
        fn set_friend_status(&self, _: &str, _: UserStatusKind) -> SetFriendStatusOutcome {
            SetFriendStatusOutcome {
                was_offline: false,
                friend_existed: true,
            }
        }
        fn set_friend_offline(&self, friend_key: &str, ts: i64) {
            self.state
                .lock()
                .calls_set_offline
                .push((friend_key.to_string(), ts));
        }
        fn set_friend_last_heartbeat(&self, _: &str, _: i64) {}
        fn set_friend_game_info(&self, _: &str, _: Option<GameInfoSnapshot>) {}
        fn set_friend_dht_record_key(&self, friend_key: &str, dht_key: &str) {
            self.state
                .lock()
                .calls_set_dht_key
                .push((dht_key.to_string(), friend_key.to_string()));
        }
        fn cache_route_blob(&self, _: &str, _: Vec<u8>) {}
        fn set_unwatched_friend(&self, friend_key: &str, unwatched: bool) {
            self.state
                .lock()
                .calls_set_unwatched
                .push((friend_key.to_string(), unwatched));
        }
        async fn acquire_friend_record(
            &self,
            dht_record_key: &str,
        ) -> Result<rekindle_records::lease::LeaseId, PresenceError> {
            let mut st = self.state.lock();
            st.calls_open_friend.push(dht_record_key.to_string());
            Ok(rekindle_records::lease::LeaseId(
                u64::try_from(st.calls_open_friend.len()).unwrap_or(u64::MAX),
            ))
        }
        async fn hold_friend_record(
            &self,
            friend_key: &str,
            lease: rekindle_records::lease::LeaseId,
        ) {
            self.state
                .lock()
                .calls_hold
                .push((friend_key.to_string(), lease.0));
        }
        async fn watch_friend_subkeys(
            &self,
            lease: rekindle_records::lease::LeaseId,
            subkeys: &[u32],
        ) -> Result<(), PresenceError> {
            self.state
                .lock()
                .calls_watch_subkeys
                .push((lease.0, subkeys.to_vec()));
            Ok(())
        }
        fn persist_friend_last_seen(&self, _: &str, _: i64) {}
        fn emit(&self, event: FriendPresenceEvent) {
            self.state.lock().calls_emit.push(event);
        }
        fn friends_with_dht_keys(&self) -> Vec<(String, String)> {
            let mut st = self.state.lock();
            st.calls_friends_with_dht += 1;
            st.friends_with_dht.clone()
        }
        fn unwatched_friends(&self) -> HashSet<String> {
            let mut st = self.state.lock();
            st.calls_unwatched += 1;
            st.unwatched.clone()
        }
        async fn fetch_friend_dht_subkey(
            &self,
            dht_record_key: &str,
            subkey: u32,
            force_refresh: bool,
        ) -> Option<Vec<u8>> {
            let mut st = self.state.lock();
            st.calls_fetch_subkey
                .push((dht_record_key.to_string(), subkey, force_refresh));
            st.subkey_bytes
                .get(&(dht_record_key.to_string(), subkey))
                .cloned()
        }
        fn find_stale_friend_heartbeats(&self, threshold_ms: i64) -> Vec<String> {
            let mut st = self.state.lock();
            st.calls_find_stale.push(threshold_ms);
            st.stale_friends.clone()
        }
    }

    #[tokio::test]
    async fn empty_friend_list_is_a_no_op() {
        let deps = Arc::new(MockDeps::default());
        let mut watched = HashSet::new();
        sync_friends(
            Arc::clone(&deps),
            &mut watched,
            false,
            false,
            &tokio_util::sync::CancellationToken::new(),
        )
        .await;
        let st = deps.state.lock();
        assert!(st.calls_set_dht_key.is_empty());
        assert!(st.calls_fetch_subkey.is_empty());
    }

    #[tokio::test]
    async fn friend_sync_registers_then_watches_then_polls() {
        let deps = Arc::new(MockDeps {
            state: Mutex::new(MockState {
                friends_with_dht: vec![("alice".into(), "dht1".into())],
                ..Default::default()
            }),
            accepted: HashSet::new(),
        });
        let mut watched = HashSet::new();
        sync_friends(
            Arc::clone(&deps),
            &mut watched,
            true,
            false,
            &tokio_util::sync::CancellationToken::new(),
        )
        .await;
        let st = deps.state.lock();
        assert_eq!(st.calls_set_dht_key, vec![("dht1".into(), "alice".into())]);
        assert_eq!(st.calls_open_friend, vec!["dht1".to_string()]);
        // The watch sits on the lease the host then keeps for the friend.
        assert_eq!(st.calls_watch_subkeys.first().map(|(l, _)| *l), Some(1));
        assert_eq!(st.calls_hold, vec![("alice".to_string(), 1)]);
        // 3 subkeys polled: status, game, route.
        let subkeys: Vec<u32> = st.calls_fetch_subkey.iter().map(|(_, sk, _)| *sk).collect();
        assert_eq!(subkeys, vec![2, 4, 6]);
        // All polled with force_refresh=true (first_tick).
        for (_, _, force) in &st.calls_fetch_subkey {
            assert!(*force);
        }
        assert!(watched.contains("dht1"));
    }

    #[tokio::test]
    async fn unwatched_friend_clears_watched_set_so_watch_retries() {
        let deps = Arc::new(MockDeps {
            state: Mutex::new(MockState {
                friends_with_dht: vec![("alice".into(), "dht1".into())],
                unwatched: ["alice".to_string()].into_iter().collect(),
                ..Default::default()
            }),
            accepted: HashSet::new(),
        });
        let mut watched: HashSet<String> = ["dht1".to_string()].into_iter().collect();
        sync_friends(
            Arc::clone(&deps),
            &mut watched,
            false,
            false,
            &tokio_util::sync::CancellationToken::new(),
        )
        .await;
        // unwatched cleared "dht1" → re-watched this tick → restored.
        assert!(watched.contains("dht1"));
        let st = deps.state.lock();
        assert_eq!(st.calls_open_friend, vec!["dht1".to_string()]);
    }

    #[tokio::test]
    async fn force_all_passes_force_refresh_true_to_every_fetch() {
        let deps = Arc::new(MockDeps {
            state: Mutex::new(MockState {
                friends_with_dht: vec![("alice".into(), "dht1".into())],
                ..Default::default()
            }),
            accepted: HashSet::new(),
        });
        let mut watched: HashSet<String> = ["dht1".to_string()].into_iter().collect();
        sync_friends(
            Arc::clone(&deps),
            &mut watched,
            false,
            true,
            &tokio_util::sync::CancellationToken::new(),
        )
        .await;
        let st = deps.state.lock();
        for (_, _, force) in &st.calls_fetch_subkey {
            assert!(*force, "force_all should propagate to every subkey fetch");
        }
    }

    #[test]
    fn check_stale_presences_emits_for_accepted_only() {
        let deps = MockDeps {
            state: Mutex::new(MockState {
                stale_friends: vec!["accepted-stale".into(), "pending-stale".into()],
                ..Default::default()
            }),
            accepted: ["accepted-stale".to_string()].into_iter().collect(),
        };
        check_stale_friend_presences(&deps);
        let st = deps.state.lock();
        // Both got set_friend_offline.
        assert_eq!(st.calls_set_offline.len(), 2);
        // Only accepted got emitted.
        assert_eq!(st.calls_emit.len(), 1);
        assert!(matches!(
            st.calls_emit[0],
            FriendPresenceEvent::FriendOffline { ref friend_key } if friend_key == "accepted-stale"
        ));
    }
}
