//! Phase 21 REDO — `FriendPresenceDeps` impl for `PresenceAdapter`.
//!
//! Owns the 20-method friend-presence surface (lookup, status
//! mutations, DHT IO, profile publish, heartbeat). Community
//! presence lives in the sibling `community_deps.rs` module.

use async_trait::async_trait;
use rekindle_presence::{
    FriendPresenceDeps, FriendPresenceEvent, GameInfoSnapshot, PresenceError,
    SetFriendStatusOutcome, UserStatusKind,
};

use crate::services::presence_adapter::mapping::{
    from_crate_game_info, from_crate_status, map_event, to_crate_status,
};
use crate::services::presence_adapter::PresenceAdapter;
use crate::state::UserStatus;
use crate::state_helpers;

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
        if let Some(api) = state_helpers::veilid_api(&self.state) {
            let mut dht_mgr = self.state.dht_manager.write();
            if let Some(mgr) = dht_mgr.as_mut() {
                mgr.manager.cache_route(&api, friend_key, blob);
            }
        }
    }

    fn track_open_record(&self, dht_record_key: &str) {
        let mut dht_mgr = self.state.dht_manager.write();
        if let Some(mgr) = dht_mgr.as_mut() {
            mgr.track_open_record(dht_record_key.to_string());
        }
    }

    fn set_unwatched_friend(&self, friend_key: &str, unwatched: bool) {
        let mut set = self.state.unwatched_friends.write();
        if unwatched {
            set.insert(friend_key.to_string());
        } else {
            set.remove(friend_key);
        }
    }

    async fn open_friend_record(&self, dht_record_key: &str) -> Result<(), PresenceError> {
        let rc = {
            let node = self.state.node.read();
            node.as_ref().map(|nh| nh.routing_context.clone())
        };
        let Some(rc) = rc else {
            // No routing context — non-fatal; the crate caller treats
            // this the same as "watch will be retried later".
            return Ok(());
        };
        let record_key: veilid_core::RecordKey =
            dht_record_key
                .parse()
                .map_err(|e: veilid_core::VeilidAPIError| {
                    PresenceError::InvalidDhtKey(e.to_string())
                })?;
        // The returned `DHTRecordDescriptor` is intentionally
        // dropped: the side effect (Veilid now tracks this record
        // for the watch + subsequent reads) is what we want; the
        // metadata struct is reconstructible at any time via
        // `inspect_dht_record`.
        rc.open_dht_record(record_key, None)
            .await
            .map(drop)
            .map_err(|e| {
                tracing::warn!(error = %e, dht_key = %dht_record_key, "failed to open DHT record");
                PresenceError::Dht(e.to_string())
            })
    }

    async fn watch_friend_subkeys(
        &self,
        dht_record_key: &str,
        subkeys: &[u32],
    ) -> Result<bool, PresenceError> {
        let rc = {
            let node = self.state.node.read();
            node.as_ref().map(|nh| nh.routing_context.clone())
        };
        let Some(rc) = rc else {
            return Err(PresenceError::NotAttached);
        };
        let record_key: veilid_core::RecordKey =
            dht_record_key
                .parse()
                .map_err(|e: veilid_core::VeilidAPIError| {
                    PresenceError::InvalidDhtKey(e.to_string())
                })?;
        let subkey_range: veilid_core::ValueSubkeyRangeSet = subkeys.iter().copied().collect();
        rc.watch_dht_values(record_key, Some(subkey_range), None, None)
            .await
            .map_err(|e| PresenceError::Dht(e.to_string()))
    }

    fn profile_dht_info(&self) -> Option<(String, Option<String>)> {
        let node = self.state.node.read();
        let nh = node.as_ref()?;
        let profile_key = nh.profile_dht_key.clone()?;
        let owner_keypair_str = nh
            .profile_owner_keypair
            .as_ref()
            .map(std::string::ToString::to_string);
        Some((profile_key, owner_keypair_str))
    }

    async fn open_profile_record_for_write(
        &self,
        profile_key: &str,
        owner_keypair_str: Option<&str>,
    ) -> Result<(), PresenceError> {
        let rc = {
            let node = self.state.node.read();
            node.as_ref().map(|nh| nh.routing_context.clone())
        };
        let Some(rc) = rc else {
            return Err(PresenceError::NotAttached);
        };
        let record_key: veilid_core::RecordKey =
            profile_key
                .parse()
                .map_err(|e: veilid_core::VeilidAPIError| {
                    PresenceError::InvalidDhtKey(e.to_string())
                })?;
        let owner_kp = match owner_keypair_str {
            Some(s) => Some(
                s.parse::<veilid_core::KeyPair>()
                    .map_err(|e| PresenceError::InvalidDhtKey(format!("owner keypair: {e}")))?,
            ),
            None => None,
        };
        rc.open_dht_record(record_key, owner_kp)
            .await
            .map(drop)
            .map_err(|e| PresenceError::Dht(e.to_string()))
    }

    async fn write_profile_status_subkey(
        &self,
        profile_key: &str,
        payload: Vec<u8>,
    ) -> Result<(), PresenceError> {
        let rc = {
            let node = self.state.node.read();
            node.as_ref().map(|nh| nh.routing_context.clone())
        };
        let Some(rc) = rc else {
            return Err(PresenceError::NotAttached);
        };
        let record_key: veilid_core::RecordKey =
            profile_key
                .parse()
                .map_err(|e: veilid_core::VeilidAPIError| {
                    PresenceError::InvalidDhtKey(e.to_string())
                })?;
        rc.set_dht_value(
            record_key,
            rekindle_presence::PROFILE_STATUS_SUBKEY,
            payload,
            None,
        )
        .await
        .map_err(|e| PresenceError::Dht(e.to_string()))?;
        Ok(())
    }

    fn persist_friend_last_seen(&self, friend_key: &str, ts_ms: i64) {
        crate::friend_repo::fire_update_last_seen_at(&self.state, &self.pool, friend_key, ts_ms);
    }

    fn current_identity_status(&self) -> Option<UserStatusKind> {
        state_helpers::identity_status(&self.state).map(to_crate_status)
    }

    fn now_ms(&self) -> i64 {
        crate::db::timestamp_now()
    }

    fn emit(&self, event: FriendPresenceEvent) {
        let payload = map_event(event);
        crate::event_dispatch::emit_live(&self.app_handle, "presence-event", &payload);
    }

    // ---- Friend-sync surface (22.c-REDO) ----

    fn friends_with_dht_keys(&self) -> Vec<(String, String)> {
        state_helpers::friends_with_dht_keys(&self.state)
    }

    fn unwatched_friends(&self) -> std::collections::HashSet<String> {
        self.state.unwatched_friends.read().clone()
    }

    async fn fetch_friend_dht_subkey(
        &self,
        dht_record_key: &str,
        subkey: u32,
        force_refresh: bool,
    ) -> Option<Vec<u8>> {
        let rc = {
            let node = self.state.node.read();
            node.as_ref().map(|nh| nh.routing_context.clone())
        }?;
        let record_key: veilid_core::RecordKey = dht_record_key.parse().ok()?;
        // Ensure the record is open (re-opening is a no-op when already open).
        if rc.open_dht_record(record_key.clone(), None).await.is_err() {
            return None;
        }
        let value = rc
            .get_dht_value(record_key, subkey, force_refresh)
            .await
            .ok()
            .flatten()?;
        let bytes = value.data().to_vec();
        if bytes.is_empty() {
            None
        } else {
            Some(bytes)
        }
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
