//! Friend, DM, and presence CLI types.
//!
//! All entity references (friend, target, conversation, message) are
//! `--flag` arguments that accept double-quoted strings.

use std::path::PathBuf;

use clap::Subcommand;

/// Friend management subcommands.
#[derive(Subcommand)]
pub enum FriendCmd {
    /// Send friend request.
    Add {
        /// Public key hex, invite link, or display name search.
        #[arg(long, short = 't')]
        target: String,
        /// Attach a message to the request.
        #[arg(long, short = 'm')]
        message: Option<String>,
    },

    /// Accept pending friend request.
    Accept {
        /// Request ID (from `rekindle friend requests`).
        #[arg(long, short = 'r')]
        request_id: String,
    },

    /// Reject pending friend request.
    Reject {
        /// Request ID.
        #[arg(long, short = 'r')]
        request_id: String,
    },

    /// List friends with presence.
    List {
        /// Filter by status: online, away, busy, offline, all.
        #[arg(long)]
        status: Option<String>,
        /// Output format override.
        #[arg(long)]
        format: Option<String>,
    },

    /// Remove a friend (with confirmation).
    Remove {
        /// Friend identifier (display name or public key).
        #[arg(long, short = 'f')]
        friend: String,
        /// Skip confirmation.
        #[arg(long)]
        yes: bool,
    },

    /// List pending inbound/outbound requests.
    Requests,

    /// Block a peer (unfriend + suppress).
    Block {
        /// Peer identifier.
        #[arg(long, short = 'f')]
        friend: String,
    },

    /// Remove a block.
    Unblock {
        /// Peer identifier.
        #[arg(long, short = 'f')]
        friend: String,
    },
}

/// Direct messaging subcommands.
#[derive(Subcommand)]
pub enum DmCmd {
    /// Send a DM.
    Send {
        /// Friend identifier (display name or public key).
        #[arg(long, short = 'f')]
        friend: String,
        /// Message body.
        #[arg(long, short = 'm')]
        message: String,
        /// Attach a file (encrypted).
        #[arg(long)]
        file: Option<PathBuf>,
    },

    /// Show recent DMs grouped by friend.
    Inbox {
        /// Filter to one friend.
        #[arg(long, short = 'f')]
        friend: Option<String>,
        /// Max messages per thread.
        #[arg(long, default_value = "50")]
        limit: usize,
        /// Messages since timestamp (ISO 8601 or epoch ms).
        #[arg(long)]
        since: Option<String>,
        /// Output format override.
        #[arg(long)]
        format: Option<String>,
    },

    /// Live-stream incoming DMs.
    Watch {
        /// Filter to one friend.
        #[arg(long, short = 'f')]
        friend: Option<String>,
    },

    /// Read a full conversation.
    Read {
        /// Conversation ID (peer public key).
        #[arg(long, short = 'i')]
        conversation_id: String,
        /// Max messages.
        #[arg(long, default_value = "50")]
        limit: usize,
        /// Load messages before this message ID.
        #[arg(long)]
        before: Option<String>,
    },
}

/// Presence management subcommands.
#[derive(Subcommand)]
pub enum PresenceCmd {
    /// Set status: online, away, busy, invisible.
    Set {
        /// Status value.
        #[arg(long, short = 's')]
        status: String,
        /// Status message.
        #[arg(long, short = 'm')]
        message: Option<String>,
        /// Game activity name.
        #[arg(long)]
        game: Option<String>,
    },

    /// Watch friend/community presence updates.
    Watch {
        /// Also watch members of a specific community.
        #[arg(long, short = 'c')]
        community: Option<String>,
    },
}
