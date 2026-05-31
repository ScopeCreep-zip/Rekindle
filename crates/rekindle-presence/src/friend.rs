//! Phase 21 REDO — friend presence orchestrators.
//!
//! Ports `src-tauri/services/presence_service.rs` (DHT value-change
//! dispatch + `watch_friend` + `publish_status`) into the crate,
//! parameterised over [`FriendPresenceDeps`]. The src-tauri side
//! collapses to a thin facade.

use std::sync::Arc;

use crate::deps::{FriendPresenceDeps, FriendPresenceEvent, GameInfoSnapshot, PresenceError};
use crate::status::UserStatusKind;

/// 2.5× the 60 s heartbeat — allows one missed heartbeat + jitter
/// before treating the peer's last status as stale + treating them as
/// offline.
pub const STALE_PRESENCE_THRESHOLD_MS: i64 = 150 * 1000;

/// Profile DHT subkey carrying the 1-byte status (legacy) or 9-byte
/// `[status, timestamp_be]` payload.
pub const PROFILE_STATUS_SUBKEY: u32 = 2;

/// Subkeys we subscribe to on a friend's profile DHT record: status
/// + game-info + route-blob.
pub const FRIEND_WATCH_SUBKEYS: &[u32] = &[2, 4, 6];

/// Parse the status byte from a payload. Accepts both the legacy
/// 1-byte format `[status]` and the new 9-byte format
/// `[status, timestamp_be]`.
#[must_use]
pub fn parse_status(data: &[u8]) -> Option<UserStatusKind> {
    if data.is_empty() {
        return None;
    }
    Some(match data[0] {
        0 => UserStatusKind::Online,
        1 => UserStatusKind::Away,
        2 => UserStatusKind::Busy,
        _ => UserStatusKind::Offline,
    })
}

/// Extract the heartbeat timestamp from the 9-byte status payload.
/// Returns `None` for legacy 1-byte payloads.
#[must_use]
pub fn parse_status_timestamp(data: &[u8]) -> Option<i64> {
    if data.len() < 9 {
        return None;
    }
    let bytes: [u8; 8] = data[1..9].try_into().ok()?;
    Some(i64::from_be_bytes(bytes))
}

/// Map a `UserStatusKind` to the wire byte that goes in profile
/// subkey 2. Invisible publishes as `3` so peers see us offline.
#[must_use]
pub fn status_to_wire_byte(status: UserStatusKind) -> u8 {
    match status {
        UserStatusKind::Online => 0,
        UserStatusKind::Away => 1,
        UserStatusKind::Busy => 2,
        UserStatusKind::Offline | UserStatusKind::Invisible => 3,
    }
}

/// Top-level dispatcher: a DHT value change arrived for one of the
/// watched friend records. Routes to per-subkey handlers.
pub fn handle_value_change<D: FriendPresenceDeps>(
    deps: &D,
    dht_key: &str,
    subkeys: &[u32],
    value: &[u8],
) {
    let Some(friend_key) = deps.friend_for_dht_key(dht_key) else {
        tracing::debug!(dht_key, "value change for unknown DHT key");
        return;
    };
    for &subkey in subkeys {
        match subkey {
            2 => handle_status_change(deps, &friend_key, value),
            4 => handle_game_change(deps, &friend_key, value),
            6 => handle_route_change(deps, &friend_key, value),
            other => tracing::trace!(subkey = other, "unhandled presence subkey change"),
        }
    }
}

fn handle_status_change<D: FriendPresenceDeps>(deps: &D, friend_key: &str, value: &[u8]) {
    let Some(mut status) = parse_status(value) else {
        return;
    };

    // Override non-offline status to offline when the timestamp is stale.
    if status != UserStatusKind::Offline {
        if let Some(ts) = parse_status_timestamp(value) {
            let now = deps.now_ms();
            if now - ts > STALE_PRESENCE_THRESHOLD_MS {
                tracing::info!(
                    friend = %friend_key,
                    age_ms = now - ts,
                    "stale presence — treating as offline",
                );
                status = UserStatusKind::Offline;
            }
        }
    }

    // Heartbeat tracking — separate from status semantics: any payload
    // carrying a timestamp updates `last_heartbeat_at`, even if the
    // status itself ends up overridden to Offline.
    if let Some(ts) = parse_status_timestamp(value) {
        deps.set_friend_last_heartbeat(friend_key, ts);
    }

    // Privacy: don't leak status from pending (un-accepted) friend
    // requests. We still persist + log it; just don't emit.
    let is_accepted = deps.is_friend_accepted(friend_key);

    if status == UserStatusKind::Offline {
        let now = deps.now_ms();
        deps.set_friend_offline(friend_key, now);
        deps.persist_friend_last_seen(friend_key, now);
        if is_accepted {
            deps.emit(FriendPresenceEvent::FriendOffline {
                friend_key: friend_key.to_string(),
            });
        }
        return;
    }

    let outcome = deps.set_friend_status(friend_key, status);
    if outcome.was_offline && outcome.friend_existed && is_accepted {
        deps.emit(FriendPresenceEvent::FriendOnline {
            friend_key: friend_key.to_string(),
        });
    }
    if is_accepted {
        deps.emit(FriendPresenceEvent::StatusChanged {
            friend_key: friend_key.to_string(),
            status,
        });
    }
}

fn handle_game_change<D: FriendPresenceDeps>(deps: &D, friend_key: &str, value: &[u8]) {
    let game = parse_game_info(value);
    deps.set_friend_game_info(friend_key, game.clone());
    if !deps.is_friend_accepted(friend_key) {
        return;
    }
    deps.emit(FriendPresenceEvent::GameChanged {
        friend_key: friend_key.to_string(),
        game,
    });
}

fn handle_route_change<D: FriendPresenceDeps>(deps: &D, friend_key: &str, value: &[u8]) {
    tracing::debug!(friend = %friend_key, "friend route blob updated");
    if !value.is_empty() {
        deps.cache_route_blob(friend_key, value.to_vec());
    }
}

fn parse_game_info(data: &[u8]) -> Option<GameInfoSnapshot> {
    if data.is_empty() {
        return None;
    }
    // Same provisional shape as src-tauri's `parse_game_info`: try a
    // direct JSON deserialization against the on-wire game-info
    // record. The capnp-encoded path lands when the rest of the
    // game-presence pipeline moves to a stricter wire format.
    #[derive(serde::Deserialize)]
    struct WireGameInfo {
        game_id: u32,
        game_name: String,
        elapsed_seconds: u32,
        #[serde(default)]
        server_address: Option<String>,
    }
    let parsed: WireGameInfo = serde_json::from_slice(data).ok()?;
    Some(GameInfoSnapshot {
        game_id: parsed.game_id,
        game_name: parsed.game_name,
        elapsed_seconds: parsed.elapsed_seconds,
        server_address: parsed.server_address,
    })
}

/// Subscribe to a friend's DHT presence record. On hard failures
/// (invalid key, open fails) the friend is added to the
/// poll-fallback set + the function returns `Ok(())` — non-fatal so
/// the calling friend-add flow proceeds.
pub async fn watch_friend<D: FriendPresenceDeps>(
    deps: Arc<D>,
    friend_key: &str,
    dht_record_key: &str,
) -> Result<(), PresenceError> {
    deps.register_friend_dht_key(dht_record_key, friend_key);
    deps.set_friend_dht_record_key(friend_key, dht_record_key);

    if let Err(error) = deps.open_friend_record(dht_record_key).await {
        tracing::warn!(
            %error,
            dht_key = %dht_record_key,
            "failed to open DHT record for watching"
        );
        deps.set_unwatched_friend(friend_key, true);
        return Ok(());
    }
    deps.track_open_record(dht_record_key);

    match deps
        .watch_friend_subkeys(dht_record_key, FRIEND_WATCH_SUBKEYS)
        .await
    {
        Ok(true) => {
            tracing::info!(
                friend = %friend_key,
                dht_key = %dht_record_key,
                "watching friend presence",
            );
            deps.set_unwatched_friend(friend_key, false);
        }
        Ok(false) => {
            tracing::warn!(
                friend = %friend_key,
                dht_key = %dht_record_key,
                "watch_dht_values returned false — adding to poll fallback set",
            );
            deps.set_unwatched_friend(friend_key, true);
        }
        Err(error) => {
            tracing::warn!(
                %error,
                friend = %friend_key,
                "failed to watch friend presence — adding to poll fallback set",
            );
            deps.set_unwatched_friend(friend_key, true);
        }
    }
    Ok(())
}

/// Publish our own status to profile subkey 2. Encodes the 9-byte
/// `[status_byte, timestamp_be]` payload + writes via the adapter.
pub async fn publish_status<D: FriendPresenceDeps>(
    deps: Arc<D>,
    status: UserStatusKind,
) -> Result<(), PresenceError> {
    let (profile_key, owner_keypair) = deps
        .profile_dht_info()
        .ok_or(PresenceError::MissingProfileKey)?;

    tracing::info!(
        ?status,
        has_owner_keypair = owner_keypair.is_some(),
        profile_key = %profile_key,
        "publish_status: writing to DHT",
    );

    deps.open_profile_record_for_write(&profile_key, owner_keypair.as_deref())
        .await?;

    let timestamp = deps.now_ms();
    let mut payload = Vec::with_capacity(9);
    payload.push(status_to_wire_byte(status));
    payload.extend_from_slice(&timestamp.to_be_bytes());

    deps.write_profile_status_subkey(&profile_key, payload)
        .await?;
    tracing::info!(?status, "published status to DHT");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_status_legacy_and_9byte_forms() {
        assert_eq!(parse_status(&[]), None);
        assert_eq!(parse_status(&[0]), Some(UserStatusKind::Online));
        assert_eq!(parse_status(&[1]), Some(UserStatusKind::Away));
        assert_eq!(parse_status(&[2]), Some(UserStatusKind::Busy));
        assert_eq!(parse_status(&[3]), Some(UserStatusKind::Offline));
        assert_eq!(parse_status(&[7]), Some(UserStatusKind::Offline));
        // Wide payload still reads byte 0.
        let mut wide = vec![1u8];
        wide.extend_from_slice(&0i64.to_be_bytes());
        assert_eq!(parse_status(&wide), Some(UserStatusKind::Away));
    }

    #[test]
    fn parse_status_timestamp_requires_9_bytes() {
        assert_eq!(parse_status_timestamp(&[0]), None);
        assert_eq!(parse_status_timestamp(&[0u8; 8]), None);
        let mut payload = vec![1u8];
        payload.extend_from_slice(&123_456_789_i64.to_be_bytes());
        assert_eq!(parse_status_timestamp(&payload), Some(123_456_789));
    }

    #[test]
    fn status_to_wire_byte_maps_invisible_as_offline() {
        assert_eq!(status_to_wire_byte(UserStatusKind::Online), 0);
        assert_eq!(status_to_wire_byte(UserStatusKind::Away), 1);
        assert_eq!(status_to_wire_byte(UserStatusKind::Busy), 2);
        assert_eq!(status_to_wire_byte(UserStatusKind::Offline), 3);
        assert_eq!(status_to_wire_byte(UserStatusKind::Invisible), 3);
    }
}
