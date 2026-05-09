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
    /// Triggered by:
    /// - Remote: gossip `MessageNotification`, DHT `ValueChange` on channel log
    /// - Local: `SubscriptionManager::emit_local()` after successful DHT write
    ///
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
        /// True when this event was emitted by the local node after a
        /// successful send — the message is from us, not from a peer.
        is_self: bool,
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
    /// A DM was sent or received in a peer conversation.
    ///
    /// Triggered by:
    /// - Remote: DhtLog watch on peer's outbound DM log (Signal-decrypted by enrichment spawn)
    /// - Local: `SubscriptionManager::emit_local()` after successful DhtLog write
    ///
    /// DM bodies are always available because DMs are decrypted at the
    /// Signal session layer before reaching SubscriptionManager.
    DirectMessageReceived {
        /// The conversation partner's public key. For both self-sent and
        /// peer-sent messages, this identifies the DM thread.
        peer_key: String,
        timestamp: u64,
        /// Display name of the message author.
        sender_name: Option<String>,
        /// Decrypted plaintext body. Always Some for DMs (Signal decrypts inline).
        body: Option<String>,
        /// True when this event was emitted by the local node after a
        /// successful send — the message is from us, not from the peer.
        is_self: bool,
    },
}
