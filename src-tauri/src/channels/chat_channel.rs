use serde::Serialize;

/// Community **channel** messages streamed to the webview.
///
/// ## Why this still exists
///
/// The rest of `ChatEvent` — calls, friend lifecycle, DMs, typing,
/// acks — has been converged onto
/// `rekindle_types::subscription_events`, so the desktop and the CLI
/// now observe one vocabulary for each of those. This variant has not,
/// and the reason is specific rather than incidental.
///
/// Tier 1 models a channel message as `ChannelMessageEvent::New`, which
/// carries `community`, `sequence` and `reply_to_sequence` — the DHT
/// metadata the daemon has when it reads a message off a channel
/// record. The desktop's five emitters do not have those: two are local
/// echoes of a message we just sent (`emit_local_chat_event`,
/// `emit_chat_event_local_impl`), and `SentChannelMessageEcho` carries
/// no community id at all.
///
/// Merging it into `DirectMessageReceived` instead was tried and
/// reverted: that variant drives `increment_dm` in the daemon's
/// `state_effects`, so routing channel messages through it would count
/// every channel message as DM unread. The two conversation kinds are
/// not distinguishable from `conversation_id` alone.
///
/// Closing this properly means threading `community_id` to the five
/// emit sites so they can produce `ChannelMessageEvent::New`, which is
/// a change to `SentChannelMessageEcho` and its callers rather than an
/// event rename.
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
}
