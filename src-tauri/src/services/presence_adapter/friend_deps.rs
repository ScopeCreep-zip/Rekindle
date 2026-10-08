//! Phase 21 REDO — `FriendPresenceDeps` impl for `PresenceAdapter`.
//!
//! Owns the 20-method friend-presence surface (lookup, status
//! mutations, DHT IO, profile publish, heartbeat). Community
//! presence lives in the sibling `community_deps.rs` module.

use async_trait::async_trait;
use rekindle_presence::{
    FriendPresenceDeps, FriendPresenceEvent, GameInfoSnapshot, PresenceError,
    SetFriendStatusOutcome, StatusPublisherDeps, UserStatusKind,
};
use rekindle_records::lease::LeaseId;

use crate::services::presence_adapter::mapping::{
    from_crate_game_info, from_crate_status, map_event, to_crate_status,
};
use crate::services::presence_adapter::PresenceAdapter;
use crate::state::UserStatus;
use crate::state_helpers;

#[async_trait]
impl StatusPublisherDeps for PresenceAdapter {
    fn profile_dht_info(&self) -> Option<String> {
        self.state.node.read().as_ref()?.profile_dht_key.clone()
    }

    async fn write_profile_status_subkey(
        &self,
        profile_key: &str,
        payload: Vec<u8>,
    ) -> Result<(), PresenceError> {
        // Present-tense: a plain write, never re-pushed late (plan C7.7j).
        let outcome = rekindle_protocol::dht::profile::set_own_profile_status(
            &*self.record_pool()?,
            profile_key,
            payload,
        )
        .await
        .map_err(|e| PresenceError::Dht(e.to_string()))?;
        if outcome.missed() {
            return Err(PresenceError::Dht(format!(
                "status not stored ({outcome:?})"
            )));
        }
        Ok(())
    }

    fn current_identity_status(&self) -> Option<UserStatusKind> {
        state_helpers::identity_status(&self.state).map(to_crate_status)
    }

    fn now_ms(&self) -> i64 {
        crate::db::timestamp_now()
    }
}

#[async_trait]
impl FriendPresenceDeps for PresenceAdapter {
    fn friend_for_dht_key(&self, dht_key: &str) -> Option<String> {
        state_helpers::friend_for_dht_key(&self.state, dht_key)
    }

    fn is_friend_accepted(&self, friend_key: &str) -> bool {
        state_helpers::is_friend_accepted(&self.state, friend_key)
    }

    fn set_friend_status(
        &self,
        friend_key: &str,
        status: UserStatusKind,
    ) -> SetFriendStatusOutcome {
        let mut friends = self.state.friends.write();
        if let Some(friend) = friends.get_mut(friend_key) {
            let was_offline = friend.status == UserStatus::Offline;
            friend.status = from_crate_status(status);
            SetFriendStatusOutcome {
                was_offline,
                friend_existed: true,
            }
        } else {
            SetFriendStatusOutcome {
                was_offline: false,
                friend_existed: false,
            }
        }
    }

    fn set_friend_offline(&self, friend_key: &str, last_seen_ts_ms: i64) {
        let mut friends = self.state.friends.write();
        if let Some(friend) = friends.get_mut(friend_key) {
            friend.status = UserStatus::Offline;
            friend.last_seen_at = Some(last_seen_ts_ms);
        }
    }

    fn set_friend_last_heartbeat(&self, friend_key: &str, heartbeat_ts_ms: i64) {
        let mut friends = self.state.friends.write();
        if let Some(friend) = friends.get_mut(friend_key) {
            friend.last_heartbeat_at = Some(heartbeat_ts_ms);
        }
    }

    fn set_friend_game_info(&self, friend_key: &str, game: Option<GameInfoSnapshot>) {
        let local = game.map(from_crate_game_info);
        let mut friends = self.state.friends.write();
        if let Some(friend) = friends.get_mut(friend_key) {
            friend.game_info = local;
        }
    }

    fn set_friend_dht_record_key(&self, friend_key: &str, dht_record_key: &str) {
        let mut friends = self.state.friends.write();
        if let Some(friend) = friends.get_mut(friend_key) {
            friend.dht_record_key = Some(dht_record_key.to_string());
        }
    }

    fn register_friend_dht_key(&self, dht_key: &str, friend_key: &str) {
        let mut dht_mgr = self.state.dht_manager.write();
        if let Some(mgr) = dht_mgr.as_mut() {
            mgr.register_friend_dht_key(dht_key.to_string(), friend_key.to_string());
        }
    }

    fn cache_route_blob(&self, friend_key: &str, blob: Vec<u8>) {
        // Single chokepoint for fresh friend routes: also fills the
        // peer_route_cache (this path previously skipped it) and heals
        // an active 1:1 call's voice roster with the new blob.
        state_helpers::cache_peer_route(&self.state, friend_key, blob);
    }

    fn set_unwatched_friend(&self, friend_key: &str, unwatched: bool) {
        let mut set = self.state.unwatched_friends.write();
        if unwatched {
            set.insert(friend_key.to_string());
        } else {
            set.remove(friend_key);
        }
    }

    async fn acquire_friend_record(&self, dht_record_key: &str) -> Result<LeaseId, PresenceError> {
        let record_key: veilid_core::RecordKey =
            dht_record_key
                .parse()
                .map_err(|e: veilid_core::VeilidAPIError| {
                    PresenceError::InvalidDhtKey(e.to_string())
                })?;
        self.record_pool()?
            .acquire(&record_key, None)
            .await
            .map_err(|e| {
                tracing::warn!(error = %e, dht_key = %dht_record_key, "failed to open DHT record");
                PresenceError::Dht(e.to_string())
            })
    }

    async fn hold_friend_record(&self, friend_key: &str, lease: LeaseId) {
        let previous = self
            .state
            .friend_leases
            .lock()
            .insert(friend_key.to_string(), lease);
        if let (Some(previous), Ok(pool)) = (previous, self.record_pool()) {
            if previous != lease {
                pool.release(previous).await;
            }
        }
    }

    async fn watch_friend_subkeys(
        &self,
        lease: LeaseId,
        subkeys: &[u32],
    ) -> Result<(), PresenceError> {
        self.record_pool()?
            .watch(lease, subkeys.iter().copied().collect())
            .await
            .map_err(|e| PresenceError::Dht(e.to_string()))
    }

    fn persist_friend_last_seen(&self, friend_key: &str, ts_ms: i64) {
        crate::friend_repo::fire_update_last_seen_at(&self.state, &self.pool, friend_key, ts_ms);
    }

    fn emit(&self, event: FriendPresenceEvent) {
        let payload =
            rekindle_types::subscription_events::SubscriptionEvent::Presence(map_event(event));
        crate::event_dispatch::emit_subscription(&self.app_handle, &payload);
    }

    // ---- Friend-sync surface (22.c-REDO) ----

    fn friends_with_dht_keys(&self) -> Vec<(String, String)> {
        state_helpers::friends_with_dht_keys(&self.state)
    }

    fn unwatched_friends(&self) -> std::collections::HashSet<String> {
        self.state.unwatched_friends.read().clone()
    }

    /// A one-shot read; the friend's held watch lease makes the borrow a
    /// table hit.
    async fn fetch_friend_dht_subkey(
        &self,
        dht_record_key: &str,
        subkey: u32,
        force_refresh: bool,
    ) -> Option<Vec<u8>> {
        let pool = self.record_pool().ok()?;
        let bytes = rekindle_protocol::dht::profile::read_profile_subkey(
            &pool,
            dht_record_key,
            subkey,
            force_refresh,
        )
        .await
        .ok()
        .flatten()?;
        (!bytes.is_empty()).then_some(bytes)
    }

    fn find_stale_friend_heartbeats(&self, threshold_ms: i64) -> Vec<String> {
        let now = crate::db::timestamp_now();
        let friends = self.state.friends.read();
        friends
            .values()
            .filter(|f| {
                f.status != crate::state::UserStatus::Offline
                    && f.last_heartbeat_at
                        .is_some_and(|ts| now - ts > threshold_ms)
            })
            .map(|f| f.public_key.clone())
            .collect()
    }
}
