//! Presence operations — status message and game presence. The status
//! itself is the session's STATUS publisher's (plan C7.8c).
//!
//! Profile subkey writes via `broadcast::dht_writes::set_own_profile_subkey`
//! (the session's record pool).

use tracing::info;

use crate::broadcast::node::TransportNode;
use crate::error::Result;
use crate::payload::dht_types::{PROFILE_SUBKEY_GAME_INFO, PROFILE_SUBKEY_STATUS_MESSAGE};
use crate::session::Session;

/// Write our status message (durable own state: held and re-pushed until
/// it lands). The status itself is written by the session's one STATUS
/// publisher (plan C7.8c).
pub async fn set_status_message(
    node: &TransportNode,
    session: &Session,
    message: &str,
) -> Result<()> {
    crate::broadcast::dht_writes::set_own_profile_subkey(
        node,
        &session.identity.profile_dht_key,
        PROFILE_SUBKEY_STATUS_MESSAGE,
        message.as_bytes().to_vec(),
    )
    .await?;
    info!("status message updated");
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
    crate::broadcast::dht_writes::set_own_profile_subkey(
        node,
        &session.identity.profile_dht_key,
        PROFILE_SUBKEY_GAME_INFO,
        bytes,
    )
    .await?;
    info!(game = game_name, "game presence updated");
    Ok(())
}

pub async fn clear_game_presence(node: &TransportNode, session: &Session) -> Result<()> {
    crate::broadcast::dht_writes::set_own_profile_subkey(
        node,
        &session.identity.profile_dht_key,
        PROFILE_SUBKEY_GAME_INFO,
        Vec::new(),
    )
    .await?;
    info!("game presence cleared");
    Ok(())
}
