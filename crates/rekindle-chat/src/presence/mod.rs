//! Presence operations — set status, game presence, heartbeat.

use std::sync::Arc;

use parking_lot::RwLock;
use rekindle_types::session_types::SessionMeta;
use rekindle_types::dht_types::{
    PROFILE_SUBKEY_STATUS, PROFILE_SUBKEY_STATUS_MESSAGE, PROFILE_SUBKEY_GAME_INFO,
    STATUS_ONLINE, STATUS_AWAY, STATUS_BUSY, STATUS_OFFLINE, STATUS_INVISIBLE,
};

use crate::io::{Confirm, PlatformIO};
use crate::time::timestamp_ms_i64;
use crate::ChatError;

pub struct PresenceService {
    pub(crate) io: Arc<PlatformIO>,
    pub(crate) session_meta: Arc<RwLock<SessionMeta>>,
}

impl PresenceService {
    pub async fn set_status(
        &self,
        status: &str,
        message: Option<&str>,
    ) -> Result<(), ChatError> {
        let identity = {
            let meta = self.session_meta.read();
            meta.identity.clone().ok_or(ChatError::NotInitialized)?
        };

        let status_byte = match status {
            "online" => STATUS_ONLINE,
            "away" => STATUS_AWAY,
            "busy" => STATUS_BUSY,
            "offline" => STATUS_OFFLINE,
            "invisible" => STATUS_INVISIBLE,
            unknown => {
                tracing::warn!(status = unknown, "unknown presence status — defaulting to online");
                STATUS_ONLINE
            }
        };

        let now = timestamp_ms_i64();

        let mut payload = Vec::with_capacity(9);
        payload.push(status_byte);
        payload.extend_from_slice(&now.to_be_bytes());

        // Presence is ephemeral — Confirm::None (fire and forget).
        // Published every 60s via heartbeat. Loss is acceptable.
        self.io.write_record(
            &identity.profile_dht_key, PROFILE_SUBKEY_STATUS,
            &payload, None, Confirm::None,
        ).await?;

        if let Some(msg) = message {
            self.io.write_record(
                &identity.profile_dht_key, PROFILE_SUBKEY_STATUS_MESSAGE,
                msg.as_bytes(), None, Confirm::None,
            ).await?;
        }

        tracing::info!(status, "presence updated");
        Ok(())
    }

    pub async fn set_game_presence(
        &self,
        game_name: &str,
        game_id: Option<u32>,
        elapsed_seconds: u32,
        server_address: Option<&str>,
    ) -> Result<(), ChatError> {
        let identity = {
            let meta = self.session_meta.read();
            meta.identity.clone().ok_or(ChatError::NotInitialized)?
        };

        let game_info = serde_json::json!({
            "game_name": game_name,
            "game_id": game_id,
            "elapsed_seconds": elapsed_seconds,
            "server_address": server_address,
        });
        let bytes = serde_json::to_vec(&game_info)
            .map_err(|e| ChatError::Serialization(format!("game presence: {e}")))?;

        self.io.write_record(
            &identity.profile_dht_key, PROFILE_SUBKEY_GAME_INFO,
            &bytes, None, Confirm::None,
        ).await?;

        tracing::info!(game = game_name, "game presence set");
        Ok(())
    }

    pub async fn clear_game_presence(&self) -> Result<(), ChatError> {
        let identity = {
            let meta = self.session_meta.read();
            meta.identity.clone().ok_or(ChatError::NotInitialized)?
        };

        self.io.write_record(
            &identity.profile_dht_key, PROFILE_SUBKEY_GAME_INFO,
            &[], None, Confirm::None,
        ).await?;

        tracing::info!("game presence cleared");
        Ok(())
    }

    pub async fn heartbeat(&self) -> Result<(), ChatError> {
        self.set_status("online", None).await
    }
}
