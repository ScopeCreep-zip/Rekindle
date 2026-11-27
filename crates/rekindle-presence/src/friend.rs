//! Phase 21 REDO — friend presence orchestrators.
//!
//! Ports `src-tauri/services/presence_service.rs` (DHT value-change
//! dispatch + `watch_friend` + `publish_status`) into the crate,
//! parameterised over [`FriendPresenceDeps`]. The src-tauri side
//! collapses to a thin facade.

use std::sync::Arc;

use crate::deps::{
    FriendPresenceDeps, FriendPresenceEvent, GameInfoSnapshot, PresenceError, StatusPublisherDeps,
};
use crate::status::UserStatusKind;

/// 2.5× the 60 s heartbeat — allows one missed heartbeat + jitter
/// before treating the peer's last status as stale + treating them as
/// offline.
pub const STALE_PRESENCE_THRESHOLD_MS: i64 = 150 * 1000;

/// Profile DHT subkey carrying the 1-byte status (legacy) or 9-byte
/// `[status, timestamp_be]` payload.
pub const PROFILE_STATUS_SUBKEY: u32 = rekindle_types::dht_layout::profile::STATUS;

/// Subkeys we subscribe to on a friend's profile DHT record: display
/// name (live A/B finding 39) + status + game-info + device list (a
/// first contact waiting for their prekey record, live finding L-FR3) +
/// route-blob.
///
/// These were written as a bare `&[2, 4, 6]`. Naming them means a
/// renumbering of the profile record moves this watch set with it —
/// the indices moved once already when the friend-inbox pair was
/// relocated to make room for the Strand Relay pool.
pub const FRIEND_WATCH_SUBKEYS: &[u32] = &[
    rekindle_types::dht_layout::profile::DISPLAY_NAME,
    rekindle_types::dht_layout::profile::STATUS,
    rekindle_types::dht_layout::profile::GAME_INFO,
    rekindle_types::dht_layout::profile::PREKEY_RECORDS,
    rekindle_types::dht_layout::profile::ROUTE_BLOB,
];

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
    // Most watched records are not a friend's: no news.
    let Some(friend_key) = deps.friend_for_dht_key(dht_key) else {
        tracing::trace!(dht_key, "value change for a record that is no friend's");
        return;
    };
    for &subkey in subkeys {
        match subkey {
            0 => handle_name_change(deps, &friend_key, value),
            2 => {
                if let Some(value) = opened(deps, &friend_key, subkey, value) {
                    handle_status_change(deps, &friend_key, &value);
                }
            }
            4 => {
                if let Some(value) = opened(deps, &friend_key, subkey, value) {
                    handle_game_change(deps, &friend_key, &value);
                }
            }
            5 => deps.prekeys_changed(&friend_key),
            6 => handle_route_change(deps, &friend_key, value),
            other => tracing::trace!(subkey = other, "unhandled presence subkey change"),
        }
    }
}

/// A friend's STATUS or GAME_INFO, opened under their friend-presence key
/// (plan E2.5). One that does not open (no key yet, an old epoch, a forged
/// or plaintext value) is dropped, never shown.
fn opened<D: FriendPresenceDeps>(
    deps: &D,
    friend_key: &str,
    subkey: u32,
    value: &[u8],
) -> Option<Vec<u8>> {
    let Some(key) = deps.friend_presence_key(friend_key) else {
        tracing::debug!(friend = %friend_key, subkey, "no presence key for friend; value dropped");
        return None;
    };
    rekindle_secrets::friend_presence::open(&key, friend_key, subkey, value)
        .map_err(|e| {
            tracing::debug!(friend = %friend_key, subkey, error = %e, "presence value dropped");
        })
        .ok()
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
    deps.set_friend_game_info(friend_key, game);
}

/// A friend's profile subkey 0: their own display name, UTF-8 as they
/// wrote it (`rekindle_protocol::dht::profile`). One that is empty, too
/// long or holds a forbidden character is dropped; a new one is stored
/// and published (live A/B finding 39).
fn handle_name_change<D: FriendPresenceDeps>(deps: &D, friend_key: &str, value: &[u8]) {
    let name = std::str::from_utf8(value)
        .ok()
        .and_then(|text| rekindle_types::presence::limits::peer_display_name(text).ok())
        .map(str::trim)
        .filter(|name| !name.is_empty());
    let Some(name) = name else {
        tracing::debug!(friend = %friend_key, "friend display name dropped");
        return;
    };
    if deps.set_friend_display_name(friend_key, name) {
        deps.emit(FriendPresenceEvent::DisplayNameChanged {
            friend_key: friend_key.to_string(),
            display_name: name.to_string(),
        });
    }
}

/// A friend's profile subkey 6: their route blob, or empty while their
/// route is down (plan RV2 F9 A3), which drops the one we cached.
fn handle_route_change<D: FriendPresenceDeps>(deps: &D, friend_key: &str, value: &[u8]) {
    if value.is_empty() {
        tracing::debug!(friend = %friend_key, "friend route withdrawn");
        deps.drop_route_blob(friend_key);
    } else {
        tracing::debug!(friend = %friend_key, "friend route blob updated");
        deps.cache_route_blob(friend_key, value.to_vec());
    }
}

/// A friend's profile subkey 4: the schema's packed Cap'n Proto
/// `GameStatus` (`schemas/presence.capnp`), empty when no game (plan
/// E1.1). A server address that is not a valid `host:port` is dropped on
/// ingest, so no peer can hand our launcher a URI of its making.
fn parse_game_info(data: &[u8]) -> Option<GameInfoSnapshot> {
    if data.is_empty() {
        return None;
    }
    let game = match rekindle_codec::capnp_codec::presence::decode_game_status(data) {
        Ok(game) => game,
        Err(e) => {
            tracing::debug!(error = %e, "friend game info does not decode");
            return None;
        }
    };
    let server_address = game.server_address.and_then(|text| {
        rekindle_types::game_server::ServerAddress::parse(&text)
            .map_err(|e| tracing::debug!(error = %e, "friend game server address dropped"))
            .ok()
            .map(String::from)
    });
    Some(GameInfoSnapshot {
        game_id: game.game_id,
        game_name: game.game_name,
        elapsed_seconds: game.elapsed_seconds,
        server_address,
    })
}

/// The one read of a friend's display name, status and game after their
/// watch is set ([`watch_friend`]); the host runs it on its own task.
pub type PresenceRead = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>;

/// Subscribe to a friend's DHT presence record. On hard failures
/// (invalid key, open fails) the friend is added to the
/// log + the function returns `Ok(())` — non-fatal so
/// the calling friend-add flow proceeds.
pub async fn watch_friend<D: FriendPresenceDeps>(
    deps: Arc<D>,
    friend_key: &str,
    dht_record_key: &str,
) -> Result<Option<PresenceRead>, PresenceError> {
    deps.set_friend_dht_record_key(friend_key, dht_record_key);

    let lease = match deps.acquire_friend_record(dht_record_key).await {
        Ok(lease) => lease,
        Err(error) => {
            tracing::warn!(
                %error,
                dht_key = %dht_record_key,
                "failed to open DHT record for watching"
            );
            return Ok(None);
        }
    };
    // Watch on the new lease before handing it over: the host releases
    // the friend's previous lease, and the watch must already sit on the
    // one it keeps.
    match deps.watch_friend_subkeys(lease, FRIEND_WATCH_SUBKEYS).await {
        Ok(()) => {
            tracing::info!(
                friend = %friend_key,
                dht_key = %dht_record_key,
                "watching friend presence",
            );
        }
        Err(error) => {
            tracing::warn!(
                %error,
                friend = %friend_key,
                "failed to watch friend presence; the watch is re-armed by the pool",
            );
        }
    }
    // Held either way: the pool re-arms a held record's watch.
    deps.hold_friend_record(friend_key, lease).await;
    // The name, status and game are read once now: a Veilid watch reports
    // changes, not the value the record already holds (`watch_dht_values`,
    // veilid-core `routing_context.rs`; live A/B finding 39), so a status the
    // friend published before our watch would stay unseen until their next
    // publish. Status and game go through `handle_value_change`, the path a
    // watched change takes, so state and events stay one path. The reads
    // are network GETs, so they are handed back for the host to run on its
    // session scope: the accept or add that set the watch never waits.
    let sealed = read_sealed_presence(Arc::clone(&deps), friend_key, dht_record_key);
    let dht_record_key = dht_record_key.to_string();
    let friend_key = friend_key.to_string();
    Ok(Some(Box::pin(async move {
        use rekindle_types::dht_layout::profile::DISPLAY_NAME;
        let (name, ()) = tokio::join!(
            deps.fetch_friend_dht_subkey(&dht_record_key, DISPLAY_NAME, true),
            sealed,
        );
        if let Some(name) = name {
            handle_name_change(&*deps, &friend_key, &name);
        }
    })))
}

/// The one read of a friend's STATUS and GAME_INFO through
/// [`handle_value_change`], the path a watched change takes. Run after the
/// watch is set ([`watch_friend`]) and again when the friend's presence key
/// is stored or moves to a new epoch (plan E2.5): a value that reached us
/// before the key it is sealed under was dropped, and a Veilid watch only
/// reports a later change (`watch_dht_values`, veilid-core
/// `routing_context.rs`), so without this read it stays unseen until the
/// friend's next publish. The value is re-read, never buffered: the record
/// holds the friend's latest, which is the only one worth showing.
pub fn read_sealed_presence<D: FriendPresenceDeps>(
    deps: Arc<D>,
    friend_key: &str,
    dht_record_key: &str,
) -> PresenceRead {
    let dht_record_key = dht_record_key.to_string();
    tracing::trace!(friend = %friend_key, "reading friend presence");
    Box::pin(async move {
        use rekindle_types::dht_layout::profile::{GAME_INFO, STATUS};
        let (status, game) = tokio::join!(
            deps.fetch_friend_dht_subkey(&dht_record_key, STATUS, true),
            deps.fetch_friend_dht_subkey(&dht_record_key, GAME_INFO, true),
        );
        if let Some(status) = status {
            handle_value_change(&*deps, &dht_record_key, &[STATUS], &status);
        }
        if let Some(game) = game {
            handle_value_change(&*deps, &dht_record_key, &[GAME_INFO], &game);
        }
    })
}

/// Publish our own status to profile subkey 2: the 9-byte
/// `[status_byte, timestamp_be]` payload, sealed under our friend-presence
/// key (plan E2.5), written via the adapter.
pub async fn publish_status<D: StatusPublisherDeps + ?Sized>(
    deps: Arc<D>,
    status: UserStatusKind,
) -> Result<(), PresenceError> {
    let profile_key = deps
        .profile_dht_info()
        .ok_or(PresenceError::MissingProfileKey)?;

    tracing::info!(
        ?status,
        profile_key = %profile_key,
        "publish_status: writing to DHT",
    );

    let timestamp = deps.now_ms();
    let mut payload = Vec::with_capacity(9);
    payload.push(status_to_wire_byte(status));
    payload.extend_from_slice(&timestamp.to_be_bytes());

    let (owner, key) = deps.presence_key().ok_or(PresenceError::NoPresenceKey)?;
    let sealed =
        rekindle_secrets::friend_presence::seal(&key, &owner, PROFILE_STATUS_SUBKEY, &payload)
            .map_err(|e| PresenceError::Seal(e.to_string()))?;
    deps.write_profile_status_subkey(&profile_key, sealed)
        .await?;
    tracing::info!(?status, "published status to DHT");
    Ok(())
}

#[cfg(test)]
#[path = "friend_tests.rs"]
mod tests;
