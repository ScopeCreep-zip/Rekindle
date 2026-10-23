//! Friend-presence staleness (plan E2.5, D23): a local TTL timer over the
//! friend projection, with no DHT read. A friend's presence arrives by
//! their profile record's watch (`watch_friend`, `handle_value_change`);
//! a friend whose last heartbeat is older than
//! [`STALE_PRESENCE_THRESHOLD_MS`] is marked offline here.

use crate::deps::{FriendPresenceDeps, FriendPresenceEvent};
use crate::friend::STALE_PRESENCE_THRESHOLD_MS;

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
    use std::collections::HashSet;
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
        subkey_bytes: HashMap<(String, u32), Vec<u8>>,
        stale_friends: Vec<String>,
        presence_keys: HashMap<String, rekindle_secrets::friend_presence::PresenceKey>,
        // Recorders
        calls_set_dht_key: Vec<(String, String)>,
        calls_open_friend: Vec<String>,
        calls_watch_subkeys: Vec<(u64, Vec<u32>)>,
        calls_fetch_subkey: Vec<(String, u32, bool)>,
        calls_set_offline: Vec<(String, i64)>,
        calls_is_accepted: Vec<String>,
        calls_emit: Vec<FriendPresenceEvent>,
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
        fn friend_presence_key(
            &self,
            friend_key: &str,
        ) -> Option<rekindle_secrets::friend_presence::PresenceKey> {
            self.state.lock().presence_keys.get(friend_key).cloned()
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
        fn set_friend_display_name(&self, _: &str, _: &str) -> bool {
            true
        }
        fn set_friend_dht_record_key(&self, friend_key: &str, dht_key: &str) {
            self.state
                .lock()
                .calls_set_dht_key
                .push((dht_key.to_string(), friend_key.to_string()));
        }
        fn cache_route_blob(&self, _: &str, _: Vec<u8>) {}
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

    /// Plan E2.5: a friend's STATUS opens under their current key only; a
    /// plaintext value, another key, an old epoch or a GAME_INFO value
    /// replayed as STATUS is dropped.
    #[test]
    fn a_status_opens_under_the_friends_key_only() {
        use rekindle_secrets::friend_presence::{seal, PresenceKey};
        let key = PresenceKey::new(2, [4; 32]);
        let deps = MockDeps {
            state: Mutex::new(MockState {
                friends_with_dht: vec![("alice".into(), "dht1".into())],
                presence_keys: HashMap::from([("alice".to_string(), key.clone())]),
                ..Default::default()
            }),
            accepted: HashSet::from(["alice".to_string()]),
        };
        let offline = [3u8];
        let change = |value: &[u8]| crate::friend::handle_value_change(&deps, "dht1", &[2], value);
        change(&offline);
        change(&seal(&PresenceKey::new(2, [5; 32]), "alice", 2, &offline).unwrap());
        change(&seal(&PresenceKey::new(1, [4; 32]), "alice", 2, &offline).unwrap());
        change(&seal(&key, "alice", 4, &offline).unwrap());
        change(&seal(&key, "bob", 2, &offline).unwrap());
        assert!(deps.state.lock().calls_set_offline.is_empty());
        change(&seal(&key, "alice", 2, &offline).unwrap());
        assert_eq!(deps.state.lock().calls_set_offline.len(), 1);
    }

    /// Plan E2.5 (D23): a friend's watch is opened on their profile record
    /// and held, and nothing is read: the watch brings each change.
    #[tokio::test]
    async fn watching_a_friend_holds_the_watch_and_reads_nothing() {
        let deps = Arc::new(MockDeps::default());
        crate::watch_friend(Arc::clone(&deps), "alice", "dht1")
            .await
            .unwrap();
        let st = deps.state.lock();
        assert_eq!(st.calls_set_dht_key, vec![("dht1".into(), "alice".into())]);
        assert_eq!(st.calls_open_friend, vec!["dht1".to_string()]);
        assert_eq!(
            st.calls_watch_subkeys,
            vec![(1, crate::FRIEND_WATCH_SUBKEYS.to_vec())]
        );
        assert_eq!(st.calls_hold, vec![("alice".to_string(), 1)]);
        assert!(st.calls_fetch_subkey.is_empty(), "no subkey is polled");
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
