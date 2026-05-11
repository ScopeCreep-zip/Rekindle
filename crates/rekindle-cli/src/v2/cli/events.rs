//! Social feature CLI types — reactions, pins, events, threads, game servers.

use clap::Subcommand;

/// Social feature subcommands.
#[derive(Subcommand)]
pub enum SocialCmd {
    /// Add a reaction to a message.
    ReactionAdd {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long, short = 'C')]
        channel: String,
        #[arg(long, short = 'i')]
        message_id: String,
        #[arg(long, short = 'e')]
        emoji: String,
    },
    /// Remove a reaction from a message.
    ReactionRemove {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long, short = 'C')]
        channel: String,
        #[arg(long, short = 'i')]
        message_id: String,
        #[arg(long, short = 'e')]
        emoji: String,
    },
    /// Create a community event.
    EventCreate {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        description: String,
        #[arg(long)]
        start_time: u64,
        #[arg(long)]
        end_time: Option<u64>,
        #[arg(long)]
        channel_id: Option<String>,
        #[arg(long)]
        max_attendees: Option<u32>,
    },
    /// Update a community event.
    EventUpdate {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long, short = 'i')]
        event_id: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        description: String,
        #[arg(long)]
        start_time: u64,
        #[arg(long)]
        end_time: Option<u64>,
        #[arg(long)]
        max_attendees: Option<u32>,
    },
    /// Delete a community event.
    EventDelete {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long, short = 'i')]
        event_id: String,
    },
    /// RSVP to a community event.
    EventRsvp {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long, short = 'i')]
        event_id: String,
        /// going, maybe, not_going.
        #[arg(long, short = 's')]
        status: String,
    },
    /// Broadcast an event reminder.
    EventRemind {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long, short = 'i')]
        event_id: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        minutes: u32,
    },
    /// Create a thread on a message.
    ThreadCreate {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long, short = 'C')]
        channel: String,
        #[arg(long, short = 'i')]
        parent_message_id: String,
        #[arg(long)]
        title: String,
        #[arg(long, default_value = "86400")]
        auto_archive_seconds: u32,
    },
    /// Post a message to a thread.
    ThreadMessage {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long)]
        thread_id: String,
        /// Pre-encrypted ciphertext (hex-encoded).
        #[arg(long)]
        ciphertext: String,
        #[arg(long)]
        mek_generation: u64,
        #[arg(long)]
        reply_to_id: Option<String>,
    },
    /// Archive or unarchive a thread.
    ThreadArchive {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long)]
        thread_id: String,
        #[arg(long)]
        archived: bool,
    },
    /// Add a game server to the community.
    GameServerAdd {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long)]
        game_id: String,
        #[arg(long)]
        label: String,
        #[arg(long)]
        address: String,
    },
    /// Remove a game server from the community.
    GameServerRemove {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long)]
        server_id: String,
    },
}
