//! Presence operations — set status, publish to DHT.
//!
//! Raw profile subkey writes via `broadcast::dht_writes::set`.

use tracing::info;

use crate::broadcast::node::TransportNode;
use crate::error::Result;
use crate::payload::dht_types::{
    PROFILE_SUBKEY_GAME_INFO, PROFILE_SUBKEY_STATUS, PROFILE_SUBKEY_STATUS_MESSAGE, STATUS_AWAY,
    STATUS_BUSY, STATUS_INVISIBLE, STATUS_OFFLINE, STATUS_ONLINE,
};
use crate::session::Session;

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
        _ => {
            tracing::warn!(status, "unknown status, defaulting to online");
            STATUS_ONLINE
        }
    };
    let mut payload = Vec::with_capacity(9);
    payload.push(status_byte);
    payload.extend_from_slice(&rekindle_utils::timestamp_ms_i64().to_be_bytes());
    crate::broadcast::dht_writes::set(
        node,
        &session.identity.profile_dht_key,
        PROFILE_SUBKEY_STATUS,
        payload,
        None,
    )
    .await?;
    if let Some(msg) = status_message {
        crate::broadcast::dht_writes::set(
            node,
            &session.identity.profile_dht_key,
            PROFILE_SUBKEY_STATUS_MESSAGE,
            msg.as_bytes().to_vec(),
            None,
        )
        .await?;
    }
    info!(status, "presence updated");
    Ok(())
}

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
        "game_id": game_id.unwrap_or(0), "game_name": game_name,
        "elapsed_seconds": elapsed_seconds, "server_address": server_address,
    });
    let bytes = serde_json::to_vec(&game_info).map_err(|e| {
        crate::error::TransportError::SerializationFailed {
            reason: format!("game presence: {e}"),
        }
    })?;
    crate::broadcast::dht_writes::set(
        node,
        &session.identity.profile_dht_key,
        PROFILE_SUBKEY_GAME_INFO,
        bytes,
        None,
    )
    .await?;
    info!(game = game_name, "game presence updated");
    Ok(())
}

pub async fn clear_game_presence(node: &TransportNode, session: &Session) -> Result<()> {
    crate::broadcast::dht_writes::set(
        node,
        &session.identity.profile_dht_key,
        PROFILE_SUBKEY_GAME_INFO,
        Vec::new(),
        None,
    )
    .await?;
    info!("game presence cleared");
    Ok(())
}
