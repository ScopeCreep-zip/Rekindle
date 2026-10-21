//! Friend, DM, and presence CLI types.
//!
//! All entity references (friend, target, conversation, message) are
//! `--flag` arguments that accept double-quoted strings.

use std::path::PathBuf;

use clap::Subcommand;

/// Friend management subcommands.
#[derive(Subcommand)]
pub enum FriendCmd {
    /// Send a friend request to the identity a friend code names.
    Add {
        /// Their friend code (`rekindle://add/...`, from `friend code`).
        code: String,
        /// Attach a message to the request.
        #[arg(long, short = 'm')]
        message: Option<String>,
    },

    /// Accept a pending friend request.
    Accept {
        /// The requester's public key (from `rekindle friend requests`).
        #[arg(long, short = 'r')]
        request_id: String,
    },

    /// Reject a pending friend request.
    Reject {
        /// The requester's public key.
        #[arg(long, short = 'r')]
        request_id: String,
    },

    /// Cancel a friend request we sent.
    Cancel {
        /// Their public key.
        #[arg(long, short = 'f')]
        friend: String,
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

    /// Remove a friend (the host asks to confirm).
    Remove {
        /// The friend's public key.
        #[arg(long, short = 'f')]
        friend: String,
    },

    /// List the requests waiting for our answer.
    Requests,

    /// Block anyone by public key (our profile moves to a new key).
    Block {
        /// Their public key.
        #[arg(long, short = 'f')]
        friend: String,
    },

    /// Remove a block.
    Unblock {
        /// Their public key.
        #[arg(long, short = 'f')]
        friend: String,
    },

    /// List everyone we blocked.
    Blocked,

    /// Set (or clear, with no `--name`) our nickname for a friend.
    Nickname {
        /// The friend's public key.
        #[arg(long, short = 'f')]
        friend: String,
        /// The nickname; omit to clear it.
        #[arg(long, short = 'n')]
        name: Option<String>,
    },

    /// Friend invite links.
    #[command(subcommand)]
    Invite(FriendInviteCmd),

    /// Friend groups.
    #[command(subcommand)]
    Group(GroupCmd),

    /// Reset our secure session with a friend (compare safety numbers
    /// out of band before resuming).
    ResetSession {
        /// The friend's public key.
        #[arg(long, short = 'f')]
        friend: String,
    },

    /// Show our safety number with a friend, its QR code, and whether
    /// their key is verified.
    SafetyNumber {
        /// The friend's public key.
        #[arg(long, short = 'f')]
        friend: String,
    },

    /// Mark a friend's key verified, after comparing the safety number
    /// with them out of band (the host asks to confirm).
    Verify {
        /// The friend's public key.
        #[arg(long, short = 'f')]
        friend: String,
    },

    /// Withdraw a friend's verification (the host asks to confirm).
    Unverify {
        /// The friend's public key.
        #[arg(long, short = 'f')]
        friend: String,
    },
}

/// Friend invite link subcommands.
#[derive(Subcommand)]
pub enum FriendInviteCmd {
    /// Make a tracked invite link.
    Create,
    /// List our open invites.
    List,
    /// Cancel an open invite.
    Cancel {
        /// The invite's id (from `rekindle friend invite list`).
        #[arg(long, short = 'i')]
        invite_id: String,
    },
    /// Add the friend an invite link names (reviewed with their key's
    /// fingerprint).
    Redeem {
        /// The invite link.
        #[arg(long, short = 'l')]
        link: String,
    },
}

/// Friend group subcommands.
#[derive(Subcommand)]
pub enum GroupCmd {
    /// Create a group.
    Create {
        #[arg(long, short = 'n')]
        name: String,
    },
    /// Rename a group.
    Rename {
        #[arg(long, short = 'g')]
        group_id: i64,
        #[arg(long, short = 'n')]
        name: String,
    },
    /// Delete a group; its friends fall back to no group.
    Delete {
        #[arg(long, short = 'g')]
        group_id: i64,
    },
    /// Move a friend into a group, or out of any with no `--group-id`.
    Move {
        /// The friend's public key.
        #[arg(long, short = 'f')]
        friend: String,
        #[arg(long, short = 'g')]
        group_id: Option<i64>,
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

    /// Read a conversation's history, newest last.
    History {
        /// The friend's public key.
        #[arg(long, short = 'f')]
        friend: String,
        /// Max messages.
        #[arg(long, default_value = "50")]
        limit: u32,
        /// Only messages sent before this time (epoch ms).
        #[arg(long)]
        before: Option<i64>,
    },

    /// Mark everything from a friend read.
    Read {
        /// The friend's public key.
        #[arg(long, short = 'f')]
        friend: String,
    },
}

/// A status the user may pick (offline is locking or logging out;
/// invisible appears offline to peers).
#[derive(Clone, Copy, clap::ValueEnum)]
pub enum PresenceStatus {
    Online,
    Away,
    Busy,
    Invisible,
}

/// Presence management subcommands.
#[derive(Subcommand)]
pub enum PresenceCmd {
    /// Set status: online, away, busy, invisible.
    Set {
        /// Status value.
        #[arg(long, short = 's', value_enum)]
        status: PresenceStatus,
    },

    /// Set the status message; an empty one clears it.
    Message {
        /// The message.
        text: String,
    },

    /// Show our own status, message, game and idle state.
    Show,

    /// Watch friend/community presence updates.
    Watch {
        /// Also watch members of a specific community.
        #[arg(long, short = 'c')]
        community: Option<String>,
    },
}
