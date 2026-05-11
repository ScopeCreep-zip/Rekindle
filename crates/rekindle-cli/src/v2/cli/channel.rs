//! Channel and voice CLI types.

use clap::Subcommand;

/// Channel operation subcommands.
#[derive(Subcommand)]
pub enum ChannelCmd {
    /// List channels in a community.
    List {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long)]
        format: Option<String>,
    },
    /// Create a channel.
    Create {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long, short = 'n')]
        name: String,
        /// text, voice, announcement, forum, stage, media, events.
        #[arg(long, default_value = "text")]
        kind: String,
        #[arg(long)]
        category: Option<String>,
        #[arg(long)]
        topic: Option<String>,
        #[arg(long)]
        slowmode: Option<u32>,
    },
    /// Delete a channel.
    Delete {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long, short = 'C')]
        channel: String,
        #[arg(long)]
        yes: bool,
    },
    /// Update a channel's properties.
    Update {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long, short = 'C')]
        channel: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        topic: Option<String>,
        #[arg(long)]
        slowmode: Option<u32>,
    },
    /// Send a message to a channel.
    Send {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long, short = 'C')]
        channel: String,
        #[arg(long, short = 'm')]
        message: String,
        #[arg(long)]
        reply_to: Option<String>,
    },
    /// Read channel message history.
    History {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long, short = 'C')]
        channel: String,
        #[arg(long, default_value = "50")]
        limit: usize,
        #[arg(long)]
        before: Option<String>,
        #[arg(long)]
        since: Option<String>,
        #[arg(long)]
        format: Option<String>,
    },
    /// Live-stream channel messages.
    Watch {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long, short = 'C')]
        channel: String,
        #[arg(long)]
        raw: bool,
    },
    /// Edit a message (own messages only).
    Edit {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long, short = 'C')]
        channel: String,
        #[arg(long, short = 'i')]
        message_id: String,
        #[arg(long, short = 'm')]
        new_body: String,
    },
    /// Delete a message.
    MessageDelete {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long, short = 'C')]
        channel: String,
        #[arg(long, short = 'i')]
        message_id: String,
    },
    /// Pin a message in a channel.
    Pin {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long, short = 'C')]
        channel: String,
        #[arg(long, short = 'i')]
        message_id: String,
    },
    /// Unpin a message from a channel.
    Unpin {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long, short = 'C')]
        channel: String,
        #[arg(long, short = 'i')]
        message_id: String,
    },
    /// Send a typing indicator to a channel.
    Typing {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long, short = 'C')]
        channel: String,
    },
}

/// Voice channel operation subcommands.
#[derive(Subcommand)]
pub enum VoiceCmd {
    /// Join a voice channel.
    Join {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long, short = 'C')]
        channel: String,
        #[arg(long)]
        muted: bool,
        #[arg(long)]
        deafened: bool,
    },
    /// Leave current voice session.
    Leave,
    /// Show current voice session participants.
    Status,
    /// Set self-mute state.
    Mute {
        /// Explicit mute state. Omit for toggle behavior (requires active session query).
        #[arg(long)]
        on: Option<bool>,
    },
    /// Set self-deafen state.
    Deafen {
        /// Explicit deafen state. Omit for toggle behavior.
        #[arg(long)]
        on: Option<bool>,
    },
}
