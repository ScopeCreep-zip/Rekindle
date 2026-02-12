//! Game-specific rich presence data.
//!
//! Provides additional context beyond "playing X" — like server info,
//! map name, game mode, etc. for supported games.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RichPresence {
    pub game_id: u32,
    pub details: Option<String>,
    pub state: Option<String>,
    pub server_ip: Option<String>,
    pub server_port: Option<u16>,
    pub map_name: Option<String>,
    pub player_count: Option<u32>,
    pub max_players: Option<u32>,
}

impl RichPresence {
    /// Create a minimal rich presence with just the game ID.
    pub fn basic(game_id: u32) -> Self {
        Self {
            game_id,
            ..Default::default()
        }
    }

    /// Create rich presence with server info (for multiplayer games).
    pub fn with_server(game_id: u32, server_ip: String, server_port: u16) -> Self {
        Self {
            game_id,
            server_ip: Some(server_ip),
            server_port: Some(server_port),
            ..Default::default()
        }
    }

    /// Serialize to JSON bytes for DHT publication.
    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    /// Deserialize from JSON bytes (from DHT).
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        serde_json::from_slice(data).ok()
    }
}
