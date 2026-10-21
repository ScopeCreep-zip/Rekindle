//! Group DM subcommands (plan E2.6).

use clap::Subcommand;

/// Group DMs: 3-8 members, no governance.
#[derive(Subcommand)]
pub enum GroupDmCmd {
    /// Create a group of you and 2-7 friends.
    Create {
        /// The group's name.
        #[arg(long, short = 'n')]
        name: String,
        /// A member's public key (repeat for each, 2-7).
        #[arg(long = "member", short = 'm', required = true)]
        members: Vec<String>,
    },
    /// Accept a pending invite.
    Accept {
        /// The group id (32 hex).
        group_id: String,
    },
    /// Decline a pending invite.
    Decline {
        /// The group id (32 hex).
        group_id: String,
    },
    /// Leave a group; every other member rotates its key.
    Leave {
        /// The group id (32 hex).
        group_id: String,
    },
    /// Send a message to a group.
    Send {
        /// The group id (32 hex).
        group_id: String,
        /// The message text.
        text: String,
    },
    /// Your groups and pending invites.
    List,
    /// A group's stored messages, oldest first.
    Messages {
        /// The group id (32 hex).
        group_id: String,
        /// Max messages.
        #[arg(long, default_value = "50")]
        limit: u32,
        /// Only messages sent before this time (epoch ms).
        #[arg(long)]
        before: Option<i64>,
    },
}
