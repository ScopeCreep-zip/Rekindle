//! Social feature events — reactions, pins, threads, scheduled events, game servers.

use serde::{Deserialize, Serialize};

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
        channel: String,
        thread_id: String,
        thread_name: String,
        creator_pseudonym: String,
    },
    /// A new message was posted in a thread.
    /// Triggered by: gossip `ControlPayload::ThreadMessage`.
    ThreadMessagePosted {
        community: String,
        thread_id: String,
        message_id: String,
        sender_pseudonym: String,
        timestamp: u64,
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
        event_id: String,
        title: String,
        start_time: u64,
    },
    /// A community event was updated.
    /// Triggered by: gossip `ControlPayload::EventUpdated`.
    EventUpdated {
        community: String,
        event_id: String,
        title: String,
    },
    /// A community event was deleted.
    /// Triggered by: gossip `ControlPayload::EventDeleted`.
    EventDeleted {
        community: String,
        event_id: String,
    },
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
        server_id: String,
        game_id: String,
        label: String,
    },
    /// A game server was removed from the community.
    /// Triggered by: gossip `ControlPayload::GameServerRemoved`.
    GameServerRemoved {
        community: String,
        server_id: String,
    },
}
