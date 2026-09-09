//! Channel message events — new messages, edits, deletions, DMs.
//!
//! Events carry decrypted message bodies when the MEK is available.
//! `body: None` means the MEK was not cached at event production time —
//! the TUI renders a placeholder and the body arrives on the next poll
//! cycle or MEK transfer event.

use serde::{Deserialize, Serialize};

/// Channel message lifecycle events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChannelMessageEvent {
    /// A new message was posted to a community channel.
    ///
    /// Triggered by: gossip `MessageNotification`, DHT `ValueChange` on channel log.
    /// Body is populated by the SubscriptionManager enrichment stage if the
    /// MEK for this channel is cached. If not, `body: None` and the TUI
    /// shows a placeholder until the MEK arrives.
    New {
        community: String,
        channel: String,
        message_id: String,
        sender_pseudonym: String,
        sequence: u64,
        timestamp: u64,
        /// Decrypted plaintext body. None if MEK unavailable at emission time.
        body: Option<String>,
        /// Parent message sequence for threaded replies.
        reply_to_sequence: Option<u64>,
    },
    /// A message was edited.
    /// Triggered by: gossip `ControlPayload::MessageEdited`.
    Edited {
        community: String,
        channel: String,
        message_id: String,
        edited_at: u64,
        /// New decrypted body after edit. None if MEK unavailable.
        body: Option<String>,
    },
    /// A message was deleted.
    /// Triggered by: gossip `ControlPayload::MessageDeleted`.
    Deleted {
        community: String,
        channel: String,
        message_id: String,
    },
    /// A new DM was received from a peer.
    ///
    /// Triggered by: `DmPayload::DirectMessage` via `InboundHandler::on_dm`.
    /// DM bodies are always available because DMs are decrypted at the
    /// Signal session layer before reaching SubscriptionManager.
    DirectMessageReceived {
        peer_key: String,
        timestamp: u64,
        /// Peer's display name if known from friend list.
        sender_name: Option<String>,
        /// Decrypted plaintext body. Always Some for DMs (Signal decrypts inline).
        body: Option<String>,
        /// Decryption failed — the body is a placeholder, not content.
        /// A frontend must not render it as the peer's words.
        decryption_failed: bool,
        /// Automod matched, so the body should render blurred behind a
        /// reveal. Distinct from `decryption_failed`: the text is real.
        automod_blurred: bool,
        /// Which conversation this belongs to, for routing to a window.
        ///
        /// For a DM this equals `peer_key`. It exists separately because
        /// the desktop routes its chat windows on it, and a group DM's
        /// conversation is its record key rather than any one peer.
        conversation_id: String,
        /// The sender's own id for the message, for dedup and acks.
        server_message_id: Option<String>,
        /// The message this replies to, if any.
        reply_to_id: Option<String>,
    },

    /// A direct message we sent was acknowledged by the peer.
    DirectMessageAcknowledged {
        /// The send timestamp in milliseconds, which is what a DM's
        /// local echo is keyed on — a DM has no server-assigned id, so
        /// this is a `u64` rather than the `String` that
        /// `server_message_id` uses for channel messages.
        message_id: u64,
    },

    /// We were invited into a direct conversation — a per-peer DM log,
    /// or a group DM.
    DirectConversationInvited {
        /// Who invited us.
        from: String,
        /// The conversation's DHT record.
        record_key: String,
        /// The initiator's pseudonym within the conversation.
        initiator_pseudonym: String,
        is_group: bool,
    },

    /// Something happened that should bring a conversation to the
    /// front — an incoming call, an accepted friend request.
    ///
    /// A UI hint rather than a protocol fact, but one every frontend
    /// wants: a TUI raises the pane, the desktop focuses the window.
    ConversationFocusRequested {
        peer_key: String,
        display_name: String,
        /// Why focus was requested, for a frontend that wants to
        /// distinguish (or ignore) some causes.
        reason: String,
    },
}
