//! Friend presence on both hosts (plan E2.1): `rekindle_presence`'s
//! `FriendPresenceDeps` over the Host's friend projection. A friend's
//! profile record is found from the projection (`dht_record_key`), so no
//! separate key-to-friend map is kept.

use async_trait::async_trait;
use rekindle_presence::{
    FriendPresenceDeps, FriendPresenceEvent, GameInfoSnapshot, PresenceError,
    SetFriendStatusOutcome, UserStatusKind,
};
use rekindle_records::lease::LeaseId;
use rekindle_types::friend::{FriendshipState, GameInfoState, UserStatus};
use rekindle_types::subscription_events::{
    FriendEvent, GameActivity, PresenceEvent, PresenceSnapshot, SubscriptionEvent,
};

use super::HostFriends;

/// A presence event as the subscription stream names it (a display name
/// is a friend event, live A/B finding 39): online, offline and status are
/// one observation; a game change leaves the status
/// unobserved, so neither overwrites what the peer never restated.
fn subscription_event(event: FriendPresenceEvent) -> SubscriptionEvent {
    let (friend_key, snapshot) = match event {
        FriendPresenceEvent::DisplayNameChanged {
            friend_key,
            display_name,
        } => {
            return SubscriptionEvent::Friend(FriendEvent::DisplayNameChanged {
                peer_key: friend_key,
                display_name,
            })
        }
        FriendPresenceEvent::FriendOnline { friend_key } => {
            (friend_key, PresenceSnapshot::status("online"))
        }
        FriendPresenceEvent::FriendOffline { friend_key } => {
            (friend_key, PresenceSnapshot::status("offline"))
        }
        FriendPresenceEvent::StatusChanged { friend_key, status } => {
            (friend_key, PresenceSnapshot::status(status.as_wire_str()))
        }
        FriendPresenceEvent::GameChanged { friend_key, game } => (
            friend_key,
            PresenceSnapshot::game(GameActivity::from_parts(
                game.as_ref().map(|g| g.game_name.clone()),
                game.as_ref().map(|g| g.game_id),
                game.as_ref().map(|g| g.elapsed_seconds),
                game.as_ref().and_then(|g| g.server_address.clone()),
            )),
        ),
    };
    SubscriptionEvent::Presence(PresenceEvent::FriendChanged {
        peer_key: friend_key,
        snapshot,
    })
}

impl HostFriends {
    /// `friend_key` was heard from again (their route changed or they came
    /// online): bring our re-asks to them forward (E2.5g).
    fn peer_heard(&self, friend_key: &str) {
        let (host, peer) = (self.clone(), friend_key.to_owned());
        rekindle_transport::operations::friend::FriendHost::scope(self).spawn_or_drop(
            "bring re-asks forward",
            async move {
                rekindle_transport::operations::dm::outbox::peer_heard(&host, &peer).await;
            },
        );
    }

    fn records(
        &self,
    ) -> Result<std::sync::Arc<dyn rekindle_records::port::RecordPort>, PresenceError> {
        self.ctx()
            .transport
            .read()
            .as_ref()
            .ok_or(PresenceError::NotAttached)?
            .require_records()
            .map_err(|_| PresenceError::NotAttached)
    }

    fn with_friend(
        &self,
        friend_key: &str,
        change: impl FnOnce(&mut rekindle_types::friend::FriendState),
    ) {
        if let Some(friend) = self
            .ctx()
            .session
            .write()
            .as_mut()
            .and_then(|s| s.friends.get_mut(friend_key))
        {
            change(friend);
        }
    }
}

#[async_trait]
impl FriendPresenceDeps for HostFriends {
    fn now_ms(&self) -> i64 {
        self.ctx().clock.now_ms_i64()
    }

    fn friend_for_dht_key(&self, dht_key: &str) -> Option<String> {
        self.ctx().session.read().as_ref().and_then(|s| {
            s.friends
                .values()
                .find(|f| f.dht_record_key.as_deref() == Some(dht_key))
                .map(|f| f.public_key.clone())
        })
    }

    fn is_friend_accepted(&self, friend_key: &str) -> bool {
        self.ctx().session.read().as_ref().is_some_and(|s| {
            s.friends
                .get(friend_key)
                .is_some_and(|f| f.friendship_state == FriendshipState::Accepted)
        })
    }

    fn friend_presence_key(
        &self,
        friend_key: &str,
    ) -> Option<rekindle_secrets::friend_presence::PresenceKey> {
        self.ctx()
            .session
            .read()
            .as_ref()
            .and_then(|s| s.friend_presence_keys.get(friend_key).cloned())
    }

    fn set_friend_status(
        &self,
        friend_key: &str,
        status: UserStatusKind,
    ) -> SetFriendStatusOutcome {
        let mut outcome = SetFriendStatusOutcome {
            was_offline: false,
            friend_existed: false,
        };
        self.with_friend(friend_key, |f| {
            outcome = SetFriendStatusOutcome {
                was_offline: f.status == UserStatus::Offline,
                friend_existed: true,
            };
            f.status = status;
        });
        outcome
    }

    fn set_friend_offline(&self, friend_key: &str, last_seen_ts_ms: i64) {
        self.with_friend(friend_key, |f| {
            f.status = UserStatus::Offline;
            f.last_seen_at = Some(last_seen_ts_ms);
        });
    }

    fn set_friend_last_heartbeat(&self, friend_key: &str, heartbeat_ts_ms: i64) {
        self.with_friend(friend_key, |f| f.last_heartbeat_at = Some(heartbeat_ts_ms));
    }

    fn set_friend_game_info(&self, friend_key: &str, game: Option<GameInfoSnapshot>) {
        let game = game.map(|g| GameInfoState {
            game_id: g.game_id,
            game_name: g.game_name,
            server_info: None,
            elapsed_seconds: g.elapsed_seconds,
            server_address: g.server_address,
        });
        self.with_friend(friend_key, |f| f.game_info = game);
    }

    fn set_friend_display_name(&self, friend_key: &str, display_name: &str) -> bool {
        let mut changed = false;
        self.with_friend(friend_key, |f| {
            if f.display_name != display_name {
                display_name.clone_into(&mut f.display_name);
                changed = true;
            }
        });
        if !changed {
            return false;
        }
        if let (Ok(owner), Ok(db)) = (self.ctx().require_owner(), self.ctx().db.current()) {
            let (pk, name) = (friend_key.to_string(), display_name.to_string());
            db.fire("friend display name", move |conn| {
                rekindle_db::repo::friends::set(
                    conn,
                    &owner,
                    &pk,
                    rekindle_db::repo::friends::Column::DisplayName,
                    name,
                )
            });
        }
        true
    }

    fn set_friend_dht_record_key(&self, friend_key: &str, dht_record_key: &str) {
        self.with_friend(friend_key, |f| {
            f.dht_record_key = Some(dht_record_key.to_string());
        });
    }

    fn cache_route_blob(&self, friend_key: &str, blob: Vec<u8>) {
        if let Some(node) = self.ctx().transport.read().as_ref() {
            node.peers().write().cache_route(friend_key, blob);
        }
        self.peer_heard(friend_key);
    }

    async fn acquire_friend_record(&self, dht_record_key: &str) -> Result<LeaseId, PresenceError> {
        self.records()?
            .acquire(dht_record_key, None)
            .await
            .map_err(|e| match e {
                rekindle_records::port::PortError::KeyText(reason) => {
                    PresenceError::InvalidDhtKey(reason)
                }
                other => PresenceError::Dht(other.to_string()),
            })
    }

    async fn hold_friend_record(&self, friend_key: &str, lease: LeaseId) {
        let previous = self
            .ctx()
            .friend_watches
            .leases
            .lock()
            .insert(friend_key.to_string(), lease);
        if let (Some(previous), Ok(pool)) = (previous, self.records()) {
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
        self.records()?
            .watch(lease, subkeys)
            .await
            .map_err(|e| PresenceError::Dht(e.to_string()))
    }

    fn persist_friend_last_seen(&self, friend_key: &str, ts_ms: i64) {
        let (Ok(owner), Ok(db)) = (self.ctx().require_owner(), self.ctx().db.current()) else {
            return;
        };
        let pk = friend_key.to_string();
        db.fire("friend last seen", move |conn| {
            rekindle_db::repo::friends::set(
                conn,
                &owner,
                &pk,
                rekindle_db::repo::friends::Column::LastSeenAt,
                ts_ms,
            )
        });
    }

    fn emit(&self, event: FriendPresenceEvent) {
        if let FriendPresenceEvent::FriendOnline { friend_key } = &event {
            self.peer_heard(friend_key);
        }
        self.ctx().publish_event(subscription_event(event));
    }

    async fn fetch_friend_dht_subkey(
        &self,
        dht_record_key: &str,
        subkey: u32,
        force_refresh: bool,
    ) -> Option<Vec<u8>> {
        let pool = self.records().ok()?;
        let bytes = rekindle_protocol::dht::profile::read_profile_subkey(
            pool.as_ref(),
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
        let now = self.ctx().clock.now_ms_i64();
        self.ctx()
            .session
            .read()
            .as_ref()
            .map(|s| {
                s.friends
                    .values()
                    .filter(|f| {
                        f.status != UserStatus::Offline
                            && f.last_heartbeat_at
                                .is_some_and(|ts| now - ts > threshold_ms)
                    })
                    .map(|f| f.public_key.clone())
                    .collect()
            })
            .unwrap_or_default()
    }
}
