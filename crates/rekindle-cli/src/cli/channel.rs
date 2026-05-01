//! Channel and voice CLI types.
//!
//! Community, channel, and message are `--flag` arguments that accept
//! double-quoted string inputs. This handles spaces, special characters,
//! and multi-word names cleanly without positional parsing ambiguity.

use clap::Subcommand;

/// Channel operation subcommands.
#[derive(Subcommand)]
pub enum ChannelCmd {
    /// List channels in a community (grouped by category).
    List {
        /// Community name or governance key.
        #[arg(long, short = 'c')]
        community: String,
        /// Output format override.
        #[arg(long)]
        format: Option<String>,
    },

    /// Create a channel.
    Create {
        /// Community name or governance key.
        #[arg(long, short = 'c')]
        community: String,
        /// Channel name.
        #[arg(long, short = 'n')]
        name: String,
        /// Channel kind: text, voice, announcement, forum.
        #[arg(long, default_value = "text")]
        kind: String,
        /// Category ID to place the channel in.
        #[arg(long)]
        category: Option<String>,
        /// Channel topic.
        #[arg(long)]
        topic: Option<String>,
        /// Slowmode cooldown in seconds (0 = disabled).
        #[arg(long)]
        slowmode: Option<u32>,
    },

    /// Delete a channel.
    Delete {
        /// Community name or governance key.
        #[arg(long, short = 'c')]
        community: String,
        /// Channel name or ID.
        #[arg(long, short = 'C')]
        channel: String,
        /// Skip confirmation.
        #[arg(long)]
        yes: bool,
    },

    /// Update a channel's properties.
    Update {
        /// Community name or governance key.
        #[arg(long, short = 'c')]
        community: String,
        /// Channel name or ID.
        #[arg(long, short = 'C')]
        channel: String,
        /// New name.
        #[arg(long)]
        name: Option<String>,
        /// New topic.
        #[arg(long)]
        topic: Option<String>,
        /// New slowmode cooldown in seconds.
        #[arg(long)]
        slowmode: Option<u32>,
    },

    /// Send a message to a channel.
    ///
    /// Example: rekindle channel send -c "my community" -C "general" -m "hello world"
    Send {
        /// Community name or governance key.
        #[arg(long, short = 'c')]
        community: String,
        /// Channel name or ID.
        #[arg(long, short = 'C')]
        channel: String,
        /// Message body.
        #[arg(long, short = 'm')]
        message: String,
        /// Reply to a specific message ID.
        #[arg(long)]
        reply_to: Option<String>,
    },

    /// Read channel message history.
    History {
        /// Community name or governance key.
        #[arg(long, short = 'c')]
        community: String,
        /// Channel name or ID.
        #[arg(long, short = 'C')]
        channel: String,
        /// Maximum number of messages.
        #[arg(long, default_value = "50")]
        limit: usize,
        /// Load messages before this message ID.
        #[arg(long)]
        before: Option<String>,
        /// Messages since timestamp (ISO 8601 or epoch ms).
        #[arg(long)]
        since: Option<String>,
        /// Output format override.
        #[arg(long)]
        format: Option<String>,
    },

    /// Live-stream channel messages (gossip + DHT watch).
    Watch {
        /// Community name or governance key.
        #[arg(long, short = 'c')]
        community: String,
        /// Channel name or ID.
        #[arg(long, short = 'C')]
        channel: String,
        /// Show raw ciphertext (debugging).
        #[arg(long)]
        raw: bool,
    },

    /// Pin a message.
    Pin {
        /// Community name or governance key.
        #[arg(long, short = 'c')]
        community: String,
        /// Channel name or ID.
        #[arg(long, short = 'C')]
        channel: String,
        /// Message ID.
        #[arg(long, short = 'i')]
        msg_id: String,
    },

    /// Unpin a message.
    Unpin {
        /// Community name or governance key.
        #[arg(long, short = 'c')]
        community: String,
        /// Channel name or ID.
        #[arg(long, short = 'C')]
        channel: String,
        /// Message ID.
        #[arg(long, short = 'i')]
        msg_id: String,
    },
}

/// Voice channel operation subcommands.
#[derive(Subcommand)]
pub enum VoiceCmd {
    /// Join a voice channel (MEK derive, session establish).
    Join {
        /// Community name or governance key.
        #[arg(long, short = 'c')]
        community: String,
        /// Voice channel name or ID.
        #[arg(long, short = 'C')]
        channel: String,
        /// Join muted.
        #[arg(long)]
        muted: bool,
        /// Join deafened.
        #[arg(long)]
        deafened: bool,
    },

    /// Leave current voice session.
    Leave,

    /// Show current voice session participants.
    Status,

    /// Toggle self-mute.
    Mute,

    /// Toggle self-deafen.
    Deafen,
}
