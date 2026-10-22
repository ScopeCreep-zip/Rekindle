//! Channel message events — new messages, edits, deletions, DMs.
//!
//! Events carry decrypted message bodies when the MEK is available.
//! `body: None` means the MEK was not cached at event production time —
//! the TUI renders a placeholder and the body arrives on the next poll
//! cycle or MEK transfer event.

use serde::{Deserialize, Serialize};

/// Channel message lifecycle events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
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
        reply_to_message_id: Option<String>,
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
    /// Triggered by: an inbound 1:1 `DirectMessage` via `InboundHandler::on_direct`.
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
        /// Whether we sent it: a window renders it as ours. True for the
        /// group DM log's echo of our own send (`rekindle-dm`, until E2.6);
        /// a 1:1 message received from a peer is never ours (E5.3 adds our
        /// other devices' sends).
        is_own: bool,
    },

    /// A direct message we sent changed delivery state, the one send-state
    /// event (D22's vocabulary, plan E2.5): `queued` once committed to the
    /// outbox, `sent` when a frame left this device, `delivered` from the
    /// peer's `DmAck`, `read` from its `ReadUpTo`, `failed` past its
    /// deadline or wedged (with Retry and Discard). Every window renders
    /// the state it names; none infers it.
    /// Triggered by: every state change of the message's outbox row.
    DmSendStatus {
        /// The conversation it belongs to (the peer's identity key).
        conversation_id: String,
        /// The message's client id, as `DmMessage.messageId` and
        /// `server_message_id` name it.
        client_id: String,
        state: crate::message::SendState,
    },

    /// The peer's DM chain retired entries we never fetched: every entry
    /// of their stream to us up to and including `through_seq` is gone
    /// (plan E2.5). Every window shows one gap marker in the conversation
    /// where it falls; none infers a gap itself.
    /// Triggered by: the inbound stream finding its next unread entry
    /// already retired from the peer's chain.
    DmHistoryGap {
        /// The peer (and the conversation: a 1:1 DM is keyed by its peer).
        peer_key: String,
        /// The last sequence of the peer's stream that is lost.
        through_seq: u64,
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
