//! Presence delegation — status, game activity, heartbeat.

use crate::ChatError;
use super::super::ChatService;

impl ChatService {
    pub async fn set_presence(
        &self, status: &str, message: Option<&str>,
    ) -> Result<(), ChatError> {
        self.presence.set_status(status, message).await
    }

    pub async fn heartbeat(&self) -> Result<(), ChatError> {
        self.presence.heartbeat().await
    }

    pub async fn set_game_presence(
        &self, game_name: &str, game_id: Option<u32>,
        elapsed_seconds: u32, server_address: Option<&str>,
    ) -> Result<(), ChatError> {
        self.presence.set_game_presence(game_name, game_id, elapsed_seconds, server_address).await
    }
}
