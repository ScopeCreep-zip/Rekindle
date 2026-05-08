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
    /// Wave 14 W14.2 — payload extended with the data every frontend
    /// needs to react identically. Backend emits the policy
    /// (`expected_local_camera`); each frontend interprets natively
    /// (Tauri starts WebCodecs capture; CLI prints "video active";
    /// TUI flips a status indicator). Per `feedback_backend_owns_policy`,
    /// no frontend should encode "video means camera" itself.
    CallConnected {
        call_id: String,
        kind: String,
        peer_key: String,
        peer_display_name: String,
        /// True iff `call.kind == CallKind::Video`. Frontends with a
        /// camera should turn it on; CLI/TUI without one ignore the
        /// field naturally.
        expected_local_camera: bool,
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
    /// Wave 13 — alerting hint from receiver: "I got the invite, I'm
    /// ringing the user now." Lets the caller's UI flip "Calling…" to
    /// "Ringing…" without waiting for the actual accept/decline.
    /// Best-effort; loss is acceptable.
    CallRinging {
        call_id: String,
    },
    /// Wave 14 W14.3 — backend asks frontends to focus a conversation.
    /// Emitted on call entry from both caller and receiver paths.
    /// Tauri opens/focuses the ChatWindow; CLI switches active
    /// conversation context; TUI navigates. Per
    /// `feedback_backend_owns_policy.md` — the policy "call entry
    /// focuses the conversation" lives ONCE in the backend's emit
    /// sites; every frontend reacts uniformly.
    ConversationFocusRequested {
        peer_key: String,
        display_name: String,
        /// "call-started" (caller-side, after CallInvite send) or
        /// "call-accepted" (either side, after CallConnected).
        /// Frontends MAY surface differently; not load-bearing.
        reason: String,
    },
    /// Wave 12 W12.6 — peer toggled their mic / camera / screen-share
    /// mid-call. Frontend's `ActiveCallPanel` / `VideoCallPanel` mount or
    /// unmount tiles in response. Sender authority: each side controls
    /// its own flags; this event only carries the SENDER's state, not
    /// the receiver's.
    #[serde(rename_all = "camelCase")]
    CallMediaStateChanged {
        call_id: String,
        audio: bool,
        video: bool,
        screen: bool,
        timestamp_ms: u64,
    },
    /// Wave 12 W12.11 — peer fired an emoji reaction during the call.
    /// Frontend floats the glyph over the call panel for ~2 s.
    #[serde(rename_all = "camelCase")]
    CallReactionReceived {
        call_id: String,
        sender: String,
        emoji: String,
        timestamp_ms: u64,
    },
    /// Wave 12 W12.9 — incoming group call. Mirrors `IncomingCall` but
    /// for 1:N: carries the full participants list so the recipient's
    /// UI can render "Alice is calling you to a group with B, C…".
    #[serde(rename_all = "camelCase")]
    IncomingGroupCall {
        call_id: String,
        from: String,
        display_name: String,
        kind: String,
        participants: Vec<String>,
        expires_at_ms: u64,
    },
    /// Wave 12 W12.9 — group call has at least one acceptor; we're now
    /// in an active group call. Frontend transitions to the grid UI.
    #[serde(rename_all = "camelCase")]
    GroupCallConnected {
        call_id: String,
    },
    /// Wave 12 W12.9 — another participant joined an in-progress
    /// group call. Frontend adds their tile to the grid.
    #[serde(rename_all = "camelCase")]
    GroupCallParticipantJoined {
        call_id: String,
        participant_pubkey: String,
    },
    /// Wave 12 W12.9 — a participant left. Frontend removes their tile
    /// and re-elects voice topology if needed.
    #[serde(rename_all = "camelCase")]
    GroupCallParticipantLeft {
        call_id: String,
        participant_pubkey: String,
        reason: String,
    },
    /// Wave 12 W12.9 — the entire group call has ended (last
    /// participant left, or the local user hung up).
    #[serde(rename_all = "camelCase")]
    GroupCallEnded {
        call_id: String,
        reason: String,
    },
}
