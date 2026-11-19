//! Channel message events — new messages, edits, deletions, DMs.
//!
//! Events carry decrypted message bodies when the MEK is available.
//! `body: None` means the MEK was not cached at event production time —
//! the TUI renders a placeholder and the body arrives on the next poll
//! cycle or MEK transfer event.

use serde::{Deserialize, Serialize};

/// The DM or group DM an event is about (plan E7.1's `context`), shaped
/// as `TypingContext`'s two conversation arms. A community channel's
/// timer is its `ChannelUpdated` governance op, not this.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ConversationContext {
    /// A 1:1 DM with `peer_key`.
    Dm { peer_key: String },
    /// A group DM (plan E2.6).
    GroupDm { group_id: String },
}

impl ConversationContext {
    /// The context of conversation `id` (the peer's key, or the group's
    /// id) of `kind`; `None` for a channel.
    #[must_use]
    pub fn of(id: &str, kind: crate::message::ConversationType) -> Option<Self> {
        match kind {
            crate::message::ConversationType::Dm => Some(Self::Dm {
                peer_key: id.to_owned(),
            }),
            crate::message::ConversationType::GroupDm => Some(Self::GroupDm {
                group_id: id.to_owned(),
            }),
            crate::message::ConversationType::Channel => None,
        }
    }

    /// The peer's key, or the group's id.
    #[must_use]
    pub fn id(&self) -> &str {
        match self {
            Self::Dm { peer_key } => peer_key,
            Self::GroupDm { group_id } => group_id,
        }
    }
}

/// Channel message lifecycle events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ChannelMessageEvent {
    /// A new message was posted to a community channel.
    ///
    /// Triggered by: the Host's read of the author's channel outbox (`M`/4).
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
        /// The message id of the parent this one replies to.
        reply_to_message_id: Option<String>,
        /// The D26 gate hid the post (an automod `BlurContent` match): it is
        /// stored with `automod_blurred` and every frontend renders it
        /// blurred.
        automod_blurred: bool,
    },
    /// A message was edited.
    /// Triggered by: an edit entry in its author's record (plan E3.3).
    Edited {
        community: String,
        channel: String,
        message_id: String,
        edited_at: u64,
        /// New decrypted body after edit. None if MEK unavailable.
        body: Option<String>,
    },
    /// A message was deleted for members (plan E3.3 E4).
    /// Triggered by: a delete entry in its author's record, or a merged
    /// moderator `AdminDelete` (M4).
    Deleted {
        community: String,
        channel: String,
        message_id: String,
        /// The moderator who removed it, shown as "removed by"; `None` for
        /// its author's own delete.
        removed_by: Option<String>,
    },
    /// An author's channel outbox reaches a seq no read of ours reached
    /// (plan E3.3, D23 "Gaps": loss is never silent): the author's entries
    /// through `through_seq` may be missing from `channel`. The Host stored
    /// the gap row (`messages.history_gap_through`) in every channel of the
    /// community we read, and publishes one event per channel; every window
    /// shows the marker where it falls, as the stored history would, without
    /// a reload. None infers a gap itself.
    /// Triggered by: a departure's outbox head no bounded gap read reached.
    ChannelHistoryGap {
        community: String,
        channel: String,
        /// The author whose outbox reaches the seq (hex pseudonym key).
        sender_pseudonym: String,
        /// The author's chain seq the gap reaches.
        through_seq: u64,
    },
    /// A read reached the author's outbox head (plan E3.3, D23): the gap
    /// rows of `sender_pseudonym` through `through_seq` were removed in every
    /// channel of `community`, as the history now holds what they stood for.
    /// Every window drops those markers.
    /// Triggered by: an outbox read reaching a departure's head.
    ChannelHistoryGapClosed {
        community: String,
        sender_pseudonym: String,
        through_seq: u64,
    },
    /// Our own post changed delivery state (plan E3.1 E2c): `sent` when
    /// its slot of the channel record settled at consensus, `failed` when
    /// the network superseded our page without it (a resend merges it
    /// back). Every window renders the state it names.
    /// `awaiting_key` while the post waits for the channel's key (plan
    /// E3.2 S7l, F9-1), `queued` when it is released to its write.
    /// Triggered by: the record pool settling the post's write; a post
    /// composed before the key, its release, or its key deadline.
    ChannelSendStatus {
        community: String,
        channel: String,
        message_id: String,
        state: crate::message::SendState,
        /// Why a `failed` post was not sent, as the user reads it; `None`
        /// when the network superseded its page.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// An attachment of a channel finished downloading to the place the
    /// user chose; the row now reads `downloaded`.
    /// Triggered by: the Host's `AttachmentDownload`.
    AttachmentDownloaded {
        community: String,
        channel: String,
        attachment_id: String,
    },
    /// A custom emoji's or sticker's bytes are in the chunk cache.
    /// Triggered by: the Host's eager expression fetch.
    ExpressionAssetReady {
        community: String,
        expression_id: String,
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

    /// The disappearing-message timer of a DM or group DM changed (plan
    /// E7.1), set by us (`set_by` our key) or a member: every window shows
    /// a timeline notice and its timer control takes `seconds` (0 = off).
    /// Triggered by: our `DisappearingSet`, or a received
    /// `DisappearingTimer` (0x0160).
    DisappearingTimerChanged {
        context: ConversationContext,
        seconds: u32,
        set_by: String,
    },

    /// Messages of a DM or group DM reached their `expire_at` and were
    /// deleted (plan E7.1): every window drops them, and the shell
    /// withdraws any OS notification it showed for them.
    /// Triggered by: the Host's expiry task.
    MessagesExpired {
        context: ConversationContext,
        client_ids: Vec<String>,
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
