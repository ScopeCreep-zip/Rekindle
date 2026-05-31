//! Domain events emitted by Phase 19 channel ops.
//!
//! The src-tauri adapter maps each variant to existing `ChatEvent` /
//! `CommunityEvent` / `NotificationEvent` variants. Defining them here
//! keeps the crate free of the Tauri channel types while preserving
//! the existing UI contract.

/// Channel-side domain events. Field shapes are minimal — adapter
/// snapshots additional context from `AppState` if a full UI payload
/// needs more (mirrors the `GovernanceRuntimeEvent` pattern).
#[derive(Debug, Clone)]
pub enum ChannelEvent {
    /// A new channel message has been sent locally — adapter emits
    /// `ChatEvent::ChannelMessageSent` with the full message body.
    MessageSent {
        community_id: String,
        channel_id: String,
        message_id: String,
        sender_pseudonym: String,
    },

    /// A channel message has been received from the mesh — adapter
    /// emits `ChatEvent::ChannelMessageReceived` with full body +
    /// notification routing if the local user is mentioned.
    MessageReceived {
        community_id: String,
        channel_id: String,
        message_id: String,
        sender_pseudonym: String,
        mentioned_local: bool,
    },

    /// A thread has been created under a parent message.
    ThreadCreated {
        community_id: String,
        channel_id: String,
        thread_id: String,
        parent_message_id: String,
    },

    /// A thread message has been sent/received.
    ThreadMessage {
        community_id: String,
        thread_id: String,
        message_id: String,
        sender_pseudonym: String,
    },

    /// A reaction was added/removed.
    ReactionChanged {
        community_id: String,
        channel_id: String,
        message_id: String,
        reactor_pseudonym: String,
        emoji: String,
        added: bool,
    },

    /// An expression (custom emoji / sticker / soundboard) was uploaded.
    ExpressionUploaded {
        community_id: String,
        expression_id: String,
        kind: String,
    },

    /// An expression was deleted.
    ExpressionDeleted {
        community_id: String,
        expression_id: String,
    },
}
