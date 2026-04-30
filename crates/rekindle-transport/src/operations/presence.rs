//! Presence operations — set status, publish to DHT.

use tracing::info;

use crate::error::Result;
use crate::node::TransportNode;
use crate::payload::dht_types::{
    PROFILE_SUBKEY_STATUS, PROFILE_SUBKEY_STATUS_MESSAGE, PROFILE_SUBKEY_GAME_INFO,
    STATUS_ONLINE, STATUS_AWAY, STATUS_BUSY, STATUS_OFFLINE, STATUS_INVISIBLE,
};
use crate::session::Session;

/// Set presence status and publish to profile DHT record.
///
/// Updates subkeys 2 (status byte + timestamp) and optionally 1 (status message).
pub async fn set_status(
    node: &TransportNode,
    session: &Session,
    status: &str,
    status_message: Option<&str>,
) -> Result<()> {
    info!(status, "setting presence");

    let status_byte = match status {
        "online" => STATUS_ONLINE,
        "away" => STATUS_AWAY,
        "busy" => STATUS_BUSY,
        "offline" => STATUS_OFFLINE,
        "invisible" => STATUS_INVISIBLE,
        unknown => {
            tracing::warn!(status = unknown, "unknown status, defaulting to online");
            STATUS_ONLINE
        }
    };

    let dht = node.dht()?;
    let profile = dht.profile();

    // Write status byte + timestamp
    let mut status_payload = Vec::with_capacity(9);
    status_payload.push(status_byte);
    status_payload.extend_from_slice(&rekindle_utils::timestamp_ms_i64().to_be_bytes());

    profile
        .set_subkey(
            &session.identity.profile_dht_key,
            PROFILE_SUBKEY_STATUS,
            status_payload,
        )
        .await?;

    // Write status message if provided
    if let Some(msg) = status_message {
        profile
            .set_subkey(
                &session.identity.profile_dht_key,
                PROFILE_SUBKEY_STATUS_MESSAGE,
                msg.as_bytes().to_vec(),
            )
            .await?;
    }

    info!(status, "presence updated");
    Ok(())
}

/// Set game presence info and publish to profile DHT record.
pub async fn set_game_presence(
    node: &TransportNode,
    session: &Session,
    game_name: &str,
    game_id: Option<u32>,
    elapsed_seconds: u32,
    server_address: Option<&str>,
) -> Result<()> {
    info!(game = game_name, "setting game presence");

    let game_info = serde_json::json!({
        "game_id": game_id.unwrap_or(0),
        "game_name": game_name,
        "elapsed_seconds": elapsed_seconds,
        "server_address": server_address,
    });

    let bytes = serde_json::to_vec(&game_info)
        .map_err(|e| crate::error::TransportError::SerializationFailed {
            reason: format!("game presence: {e}"),
        })?;

    let dht = node.dht()?;
    dht.profile()
        .set_subkey(
            &session.identity.profile_dht_key,
            PROFILE_SUBKEY_GAME_INFO,
            bytes,
        )
        .await?;

    info!(game = game_name, "game presence updated");
    Ok(())
}

/// Clear game presence (set subkey to empty).
pub async fn clear_game_presence(
    node: &TransportNode,
    session: &Session,
) -> Result<()> {
    let dht = node.dht()?;
    dht.profile()
        .set_subkey(
            &session.identity.profile_dht_key,
            PROFILE_SUBKEY_GAME_INFO,
            Vec::new(),
        )
        .await?;

    info!("game presence cleared");
    Ok(())
}
