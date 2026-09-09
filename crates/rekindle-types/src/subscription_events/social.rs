//! Social feature events — reactions, pins, threads, scheduled events, game servers.
//!
//! ## Why the event/thread/server variants carry whole types
//!
//! They used to carry hand-picked fragments — an event's id, title and
//! start time, but not its description, location, recurrence or RSVPs.
//! Both producers already hold the whole thing: the gossip decoder
//! receives a full `EventInfo` and was destructuring three fields out
//! of it, and the desktop's `CommunityEvent` shipped the whole DTO,
//! which is a type alias for this very type. So the fragment was pure
//! loss on one side and a needless divergence on the other.
//!
//! The three are `Box`ed: unboxed, `EventInfo`'s RSVP list and
//! recurrence rule pushed `SubscriptionEvent` to 352 bytes against a
//! 52-byte sibling in `IpcResponse`, which clippy rejects as a
//! size-imbalanced enum. A `Box<T>` serializes identically to `T`, so
//! neither wire changes.
//!
//! ## Wire constraints
//!
//! Same as [`super::presence`]: postcard on the daemon IPC, so no
//! `#[serde(flatten)]` and no `tag = "..."` enums.

use serde::{Deserialize, Serialize};

use crate::event::EventInfo;
use crate::game_server::GameServerInfo;
use crate::thread::ThreadInfo;

/// Social feature events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SocialEvent {
    // ── Reactions ───────────────────────────────────────────────
    /// A reaction was added to a message.
    /// Triggered by: gossip `ControlPayload::ReactionAdded`.
    ReactionAdded {
        community: String,
        channel: String,
        message_id: String,
        emoji: String,
        reactor_pseudonym: String,
    },
    /// A reaction was removed from a message.
    /// Triggered by: gossip `ControlPayload::ReactionRemoved`.
    ReactionRemoved {
        community: String,
        channel: String,
        message_id: String,
        emoji: String,
        reactor_pseudonym: String,
    },

    // ── Pins ────────────────────────────────────────────────────
    /// A message was pinned.
    /// Triggered by: gossip `ControlPayload::MessagePinned`.
    MessagePinned {
        community: String,
        channel: String,
        message_id: String,
        pinned_by: String,
    },
    /// A message was unpinned.
    /// Triggered by: gossip `ControlPayload::MessageUnpinned`.
    MessageUnpinned {
        community: String,
        channel: String,
        message_id: String,
    },

    // ── Threads ─────────────────────────────────────────────────
    /// A new thread was created.
    /// Triggered by: gossip `ControlPayload::ThreadCreated`.
    ThreadCreated {
        community: String,
        thread: Box<ThreadInfo>,
    },
    /// A new message was posted in a thread.
    /// Triggered by: gossip `ControlPayload::ThreadMessage`.
    ThreadMessagePosted {
        community: String,
        thread_id: String,
        message_id: String,
        sender_pseudonym: String,
        timestamp: u64,
        /// The message text, when the emitter had it in plaintext.
        ///
        /// `None` from the gossip decoder, which sees `ciphertext` and
        /// a `mek_generation` rather than a body — same shape as
        /// [`super::ChannelMessageEvent::DirectMessageReceived::body`]
        /// and for the same reason. The desktop decrypts before
        /// emitting and fills it.
        body: Option<String>,
        reply_to_id: Option<String>,
    },
    /// A thread was archived or unarchived.
    /// Triggered by: gossip `ControlPayload::ThreadArchived`.
    ThreadArchiveChanged {
        community: String,
        thread_id: String,
        archived: bool,
    },

    // ── Scheduled events ────────────────────────────────────────
    /// A community event was created.
    /// Triggered by: gossip `ControlPayload::EventCreated`.
    EventCreated {
        community: String,
        event: Box<EventInfo>,
    },
    /// A community event was updated.
    /// Triggered by: gossip `ControlPayload::EventUpdated`.
    EventUpdated {
        community: String,
        event: Box<EventInfo>,
    },
    /// A community event was deleted.
    /// Triggered by: gossip `ControlPayload::EventDeleted`.
    EventDeleted { community: String, event_id: String },
    /// Someone RSVP'd to a community event.
    /// Triggered by: gossip `ControlPayload::EventRsvpChanged`.
    EventRsvpChanged {
        community: String,
        event_id: String,
        pseudonym: String,
        rsvp_status: String,
    },
    /// A community event is starting soon.
    /// Triggered by: gossip `ControlPayload::EventReminder`.
    EventReminder {
        community: String,
        event_id: String,
        title: String,
        minutes_until_start: u32,
    },

    // ── Game servers ────────────────────────────────────────────
    /// A game server was added to the community.
    /// Triggered by: gossip `ControlPayload::GameServerAdded`.
    GameServerAdded {
        community: String,
        server: Box<GameServerInfo>,
    },
    /// A game server was removed from the community.
    /// Triggered by: gossip `ControlPayload::GameServerRemoved`.
    GameServerRemoved {
        community: String,
        server_id: String,
    },
}
