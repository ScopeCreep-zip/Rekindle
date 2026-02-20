use crate::dht::profile::{SUBKEY_GAME_INFO, SUBKEY_ROUTE_BLOB, SUBKEY_STATUS};
use crate::dht::DHTManager;
use crate::error::ProtocolError;

/// The subkeys we watch for friend presence updates.
pub const PRESENCE_WATCH_SUBKEYS: &[u32] = &[SUBKEY_STATUS, SUBKEY_GAME_INFO, SUBKEY_ROUTE_BLOB];

/// Start watching a friend's profile DHT record for presence changes.
///
/// Returns `true` if the watch is active, `false` if it was cancelled.
pub async fn watch_friend_presence(
    dht: &DHTManager,
    profile_key: &str,
) -> Result<bool, ProtocolError> {
    dht.watch_record(profile_key, PRESENCE_WATCH_SUBKEYS).await
}

/// Publish our status to DHT (subkey 2).
/// Status: 0 = online, 1 = away, 2 = busy, 3 = offline.
/// Writes a 9-byte payload: `[status_byte, timestamp_ms_be(8)]`.
pub async fn publish_status(
    dht: &DHTManager,
    profile_key: &str,
    status: u8,
    timestamp_ms: i64,
) -> Result<(), ProtocolError> {
    let mut payload = Vec::with_capacity(9);
    payload.push(status);
    payload.extend_from_slice(&timestamp_ms.to_be_bytes());
    dht.set_value(profile_key, SUBKEY_STATUS, payload).await
}

/// Publish our game info to DHT (subkey 4).
pub async fn publish_game_info(
    dht: &DHTManager,
    profile_key: &str,
    game_info: Option<&[u8]>,
) -> Result<(), ProtocolError> {
    let value = game_info.map(<[u8]>::to_vec).unwrap_or_default();
    dht.set_value(profile_key, SUBKEY_GAME_INFO, value).await
}

/// Publish our route blob to DHT (subkey 6).
pub async fn publish_route_blob(
    dht: &DHTManager,
    profile_key: &str,
    route_blob: Vec<u8>,
) -> Result<(), ProtocolError> {
    dht.set_value(profile_key, SUBKEY_ROUTE_BLOB, route_blob)
        .await
}
