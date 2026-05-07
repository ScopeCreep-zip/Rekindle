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
    /// Emitted when an inbound DM invite is received (architecture §27).
    /// The frontend prompts the user to accept or decline, which calls
    /// `accept_dm_invite` / `decline_dm_invite` Tauri commands.
    #[serde(rename_all = "camelCase")]
    DirectMessageInvite {
        from: String,
        record_key: String,
        initiator_pseudonym: String,
        is_group: bool,
    },
    /// Plan §Failure 5 — incoming direct call. The frontend mounts an
    /// IncomingCallModal and dispatches `accept_dm_call` /
    /// `decline_dm_call`; the modal closes when the modal-side timer
    /// reaches `expires_at_ms` or the user picks an action.
    #[serde(rename_all = "camelCase")]
    IncomingCall {
        call_id: String,
        from: String,
        display_name: String,
        kind: String,
        expires_at_ms: u64,
    },
    /// Plan §Failure 5 — emitted on both sides once the X25519
    /// handshake completes and the voice transport has been started.
    #[serde(rename_all = "camelCase")]
    CallConnected {
        call_id: String,
    },
    /// Plan §Failure 5 — outgoing call's 30 s timer expired without an
    /// accept. Frontend shows a toast and clears the outgoing-call
    /// state from `calls.store`.
    #[serde(rename_all = "camelCase")]
    CallTimedOut {
        call_id: String,
    },
    /// Plan §Failure 5 — incoming call ring expired locally without
    /// the user answering. Frontend updates the missed-call badge.
    #[serde(rename_all = "camelCase")]
    CallMissed {
        call_id: String,
        from: String,
    },
    /// Plan §Failure 5 — peer declined the call (received as the
    /// `app_call` reply on the caller side).
    #[serde(rename_all = "camelCase")]
    CallDeclined {
        call_id: String,
        reason: String,
    },
    /// C2 hangup — an Active call was ended (locally via the hangup
    /// button, or remotely via the peer's CallEnd payload). Frontend
    /// clears `callsState.activeCall`. Distinct from `CallTimedOut`
    /// (which fires before the call ever connected) and `CallDeclined`
    /// (the inline app_call reply rejecting the offer).
    #[serde(rename_all = "camelCase")]
    CallEnded {
        call_id: String,
        reason: String,
    },
}
