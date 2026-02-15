use serde::Serialize;

/// Events streamed from Rust to the frontend for chat operations.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "type", content = "data")]
pub enum ChatEvent {
    MessageReceived {
        from: String,
        body: String,
        timestamp: u64,
        conversation_id: String,
    },
    TypingIndicator {
        from: String,
        typing: bool,
    },
    MessageAck {
        message_id: u64,
    },
    FriendRequest {
        from: String,
        display_name: String,
        message: String,
    },
    FriendRequestAccepted {
        from: String,
        display_name: String,
    },
    FriendRequestRejected {
        from: String,
    },
    /// Emitted when a friend is added to our local list (not an incoming request).
    FriendAdded {
        public_key: String,
        display_name: String,
    },
    /// Emitted when background server fetch completes with channel history.
    ChannelHistoryLoaded {
        channel_id: String,
        messages: Vec<crate::commands::chat::Message>,
    },
}
