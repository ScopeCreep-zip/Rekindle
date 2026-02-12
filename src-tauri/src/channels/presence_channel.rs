use serde::Serialize;

/// Events streamed from Rust to the frontend for presence updates.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "type", content = "data")]
pub enum PresenceEvent {
    FriendOnline {
        public_key: String,
    },
    FriendOffline {
        public_key: String,
    },
    StatusChanged {
        public_key: String,
        status: String,
        status_message: Option<String>,
    },
    GameChanged {
        public_key: String,
        game_name: Option<String>,
        game_id: Option<u32>,
        elapsed_seconds: Option<u32>,
    },
}
