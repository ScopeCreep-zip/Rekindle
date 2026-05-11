//! Friend, DM, and presence CLI types.

use std::path::PathBuf;
use clap::Subcommand;

/// Friend management subcommands.
#[derive(Subcommand)]
pub enum FriendCmd {
    /// Send friend request.
    Add {
        #[arg(long, short = 't')]
        target: String,
        #[arg(long, short = 'm')]
        message: Option<String>,
    },
    /// Accept pending friend request.
    Accept {
        #[arg(long, short = 'r')]
        request_id: String,
    },
    /// Reject pending friend request.
    Reject {
        #[arg(long, short = 'r')]
        request_id: String,
    },
    /// List friends with presence.
    List {
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        format: Option<String>,
    },
    /// Remove a friend.
    Remove {
        #[arg(long, short = 'f')]
        friend: String,
        #[arg(long)]
        yes: bool,
    },
    /// List pending inbound/outbound requests.
    Requests,
    /// Block a peer (unfriend + suppress).
    Block {
        #[arg(long, short = 'f')]
        friend: String,
    },
    /// Remove a block.
    Unblock {
        #[arg(long, short = 'f')]
        friend: String,
    },
}

/// Direct messaging subcommands.
#[derive(Subcommand)]
pub enum DmCmd {
    /// Send a DM.
    Send {
        #[arg(long, short = 'f')]
        friend: String,
        #[arg(long, short = 'm')]
        message: String,
        #[arg(long)]
        file: Option<PathBuf>,
    },
    /// Show recent DMs grouped by friend.
    Inbox {
        #[arg(long, short = 'f')]
        friend: Option<String>,
        #[arg(long, default_value = "50")]
        limit: usize,
        #[arg(long)]
        since: Option<String>,
        #[arg(long)]
        format: Option<String>,
    },
    /// Live-stream incoming DMs.
    Watch {
        #[arg(long, short = 'f')]
        friend: Option<String>,
    },
    /// Read a full conversation with a specific peer.
    Read {
        #[arg(long, short = 'i')]
        conversation_id: String,
        #[arg(long, default_value = "50")]
        limit: usize,
        #[arg(long)]
        before: Option<String>,
    },
    /// Send a typing indicator to a DM peer.
    Typing {
        #[arg(long, short = 'f')]
        friend: String,
        /// true = started typing, false = stopped.
        #[arg(long, default_value = "true")]
        typing: bool,
    },
}

/// Presence management subcommands.
#[derive(Subcommand)]
pub enum PresenceCmd {
    /// Set status: online, away, busy, invisible.
    Set {
        #[arg(long, short = 's')]
        status: String,
        #[arg(long, short = 'm')]
        message: Option<String>,
        #[arg(long)]
        game: Option<String>,
    },
    /// Set game presence.
    Game {
        #[arg(long)]
        game_name: String,
        #[arg(long)]
        game_id: Option<u32>,
        #[arg(long)]
        elapsed_seconds: u32,
        #[arg(long)]
        server_address: Option<String>,
    },
    /// Clear game presence.
    GameClear,
    /// Watch friend/community presence updates.
    Watch {
        #[arg(long, short = 'c')]
        community: Option<String>,
    },
}
