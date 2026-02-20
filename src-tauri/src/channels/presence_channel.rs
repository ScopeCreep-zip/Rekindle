use serde::Serialize;

/// Events streamed from Rust to the frontend for presence updates.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "type", content = "data")]
pub enum PresenceEvent {
    #[serde(rename_all = "camelCase")]
    FriendOnline { public_key: String },
    #[serde(rename_all = "camelCase")]
    FriendOffline { public_key: String },
    #[serde(rename_all = "camelCase")]
    StatusChanged {
        public_key: String,
        status: String,
        status_message: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    GameChanged {
        public_key: String,
        game_name: Option<String>,
        game_id: Option<u32>,
        elapsed_seconds: Option<u32>,
    },
}
