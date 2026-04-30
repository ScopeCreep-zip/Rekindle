use serde::Serialize;

/// Events streamed from Rust to the frontend for chat operations.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "type", content = "data")]
pub enum ChatEvent {
    #[serde(rename_all = "camelCase")]
    MessageReceived {
        from: String,
        body: String,
        #[serde(default)]
        decryption_failed: bool,
        #[serde(default)]
        automod_blurred: bool,
        timestamp: u64,
        conversation_id: String,
        /// Message ID (present for community channel messages, absent for DMs).
        #[serde(skip_serializing_if = "Option::is_none")]
        server_message_id: Option<String>,
        /// ID of the message this is a reply to (community messages only).
        #[serde(skip_serializing_if = "Option::is_none")]
        reply_to_id: Option<String>,
        /// Resolved display name of the sender (community messages only).
        /// Avoids the frontend needing to resolve pseudonym keys to names.
        #[serde(skip_serializing_if = "Option::is_none")]
        sender_display_name: Option<String>,
    },
    TypingIndicator {
        from: String,
        typing: bool,
    },
    #[serde(rename_all = "camelCase")]
    MessageAck {
        message_id: u64,
    },
    #[serde(rename_all = "camelCase")]
    FriendRequest {
        from: String,
        display_name: String,
        message: String,
    },
    #[serde(rename_all = "camelCase")]
    FriendRequestAccepted {
        from: String,
        display_name: String,
    },
    FriendRequestRejected {
        from: String,
    },
    /// Emitted when a friend is added to our local list (not an incoming request).
    #[serde(rename_all = "camelCase")]
    FriendAdded {
        public_key: String,
        display_name: String,
        friendship_state: String,
    },
    /// Emitted when a friend is removed (unfriended or blocked).
    #[serde(rename_all = "camelCase")]
    FriendRemoved {
        public_key: String,
    },
    /// Emitted when a sent friend request was confirmed received by the peer.
    #[serde(rename_all = "camelCase")]
    FriendRequestDelivered {
        to: String,
    },
}
