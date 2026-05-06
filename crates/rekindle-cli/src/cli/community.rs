//! Community, role, and moderation CLI types.
//!
//! All entity references (community, member, role, invite) are `--flag`
//! arguments that accept double-quoted strings. This handles spaces,
//! special characters, and multi-word names cleanly.

use std::path::PathBuf;

use clap::Subcommand;

/// Community lifecycle subcommands.
#[derive(Subcommand)]
pub enum CommunityCmd {
    /// Create a new community.
    Create {
        /// Community name.
        name: String,
        /// Community description.
        #[arg(long)]
        description: Option<String>,
        /// Community icon image path.
        #[arg(long)]
        icon: Option<PathBuf>,
    },

    /// Join via invite code or governance key.
    Join {
        /// Invite code or governance key.
        #[arg(long, short = 'i')]
        invite: String,
        /// Override display name for this community.
        #[arg(long)]
        display_name: Option<String>,
    },

    /// Leave a community.
    Leave {
        /// Community name or governance key.
        #[arg(long, short = 'c')]
        community: String,
        /// Skip confirmation.
        #[arg(long)]
        yes: bool,
    },

    /// List joined communities.
    List {
        /// Output format override.
        #[arg(long)]
        format: Option<String>,
    },

    /// Show community details (metadata, channels, roles, members).
    Info {
        /// Community name or governance key.
        #[arg(long, short = 'c')]
        community: String,
        /// Include full channel list and role permissions.
        #[arg(long)]
        verbose: bool,
    },

    /// Approve a pending member from the waiting room.
    Approve {
        /// Community name or governance key.
        #[arg(long, short = 'c')]
        community: String,
        /// Member pseudonym to approve.
        #[arg(long, short = 'M')]
        member: String,
    },

    /// Reject a pending member from the waiting room.
    Reject {
        /// Community name or governance key.
        #[arg(long, short = 'c')]
        community: String,
        /// Member pseudonym to reject.
        #[arg(long, short = 'M')]
        member: String,
        /// Reason for rejection.
        #[arg(long)]
        reason: Option<String>,
    },

    /// List pending join requests.
    Pending {
        /// Community name or governance key.
        #[arg(long, short = 'c')]
        community: String,
    },

    /// Transfer community ownership to another member.
    Transfer {
        /// Community name or governance key.
        #[arg(long, short = 'c')]
        community: String,
        /// New owner's pseudonym key.
        #[arg(long, short = 'M')]
        new_owner: String,
        /// Skip confirmation.
        #[arg(long)]
        yes: bool,
    },

    /// Invite management.
    #[command(subcommand)]
    Invite(InviteCmd),
}

/// Invite subcommands.
#[derive(Subcommand)]
pub enum InviteCmd {
    /// Generate invite code/link.
    Create {
        /// Community name or governance key.
        #[arg(long, short = 'c')]
        community: String,
        /// Maximum number of uses (0 = unlimited).
        #[arg(long)]
        max_uses: Option<u32>,
        /// Expiration duration (e.g., "24h", "7d", "never").
        #[arg(long)]
        expires: Option<String>,
    },

    /// List active invites.
    List {
        /// Community name or governance key.
        #[arg(long, short = 'c')]
        community: String,
    },

    /// Revoke an invite.
    Revoke {
        /// Invite code to revoke.
        #[arg(long, short = 'i')]
        invite_code: String,
    },
}

/// Role management subcommands (requires MANAGE_ROLES permission).
#[derive(Subcommand)]
pub enum RoleCmd {
    /// List roles with permissions.
    List {
        /// Community name or governance key.
        #[arg(long, short = 'c')]
        community: String,
    },

    /// Create a role.
    Create {
        /// Community name or governance key.
        #[arg(long, short = 'c')]
        community: String,
        /// Role name.
        #[arg(long, short = 'n')]
        name: String,
        /// Permission bitmask (decimal or hex with 0x prefix).
        #[arg(long)]
        permissions: Option<String>,
        /// Role color (hex, e.g., "FF5733").
        #[arg(long)]
        color: Option<String>,
        /// Position in role hierarchy.
        #[arg(long)]
        position: Option<u32>,
    },

    /// Update a role.
    Update {
        /// Community name or governance key.
        #[arg(long, short = 'c')]
        community: String,
        /// Role ID.
        #[arg(long, short = 'r')]
        role_id: String,
        /// New name.
        #[arg(long)]
        name: Option<String>,
        /// New permission bitmask.
        #[arg(long)]
        permissions: Option<String>,
        /// New color.
        #[arg(long)]
        color: Option<String>,
    },

    /// Delete a role.
    Delete {
        /// Community name or governance key.
        #[arg(long, short = 'c')]
        community: String,
        /// Role ID.
        #[arg(long, short = 'r')]
        role_id: String,
        /// Skip confirmation.
        #[arg(long)]
        yes: bool,
    },

    /// Assign a role to a member.
    Assign {
        /// Community name or governance key.
        #[arg(long, short = 'c')]
        community: String,
        /// Member pseudonym or display name.
        #[arg(long, short = 'M')]
        member: String,
        /// Role ID.
        #[arg(long, short = 'r')]
        role_id: String,
    },

    /// Remove a role from a member.
    Unassign {
        /// Community name or governance key.
        #[arg(long, short = 'c')]
        community: String,
        /// Member pseudonym or display name.
        #[arg(long, short = 'M')]
        member: String,
        /// Role ID.
        #[arg(long, short = 'r')]
        role_id: String,
    },
}

/// Moderation subcommands (requires moderation permissions).
#[derive(Subcommand)]
pub enum ModerateCmd {
    /// Kick a member from the community.
    Kick {
        /// Community name or governance key.
        #[arg(long, short = 'c')]
        community: String,
        /// Member pseudonym or display name.
        #[arg(long, short = 'M')]
        member: String,
        /// Reason for the kick.
        #[arg(long)]
        reason: Option<String>,
    },

    /// Ban a member from the community.
    Ban {
        /// Community name or governance key.
        #[arg(long, short = 'c')]
        community: String,
        /// Member pseudonym or display name.
        #[arg(long, short = 'M')]
        member: String,
        /// Reason for the ban.
        #[arg(long)]
        reason: Option<String>,
    },

    /// Unban a member.
    Unban {
        /// Community name or governance key.
        #[arg(long, short = 'c')]
        community: String,
        /// Member pseudonym or display name.
        #[arg(long, short = 'M')]
        member: String,
    },

    /// Timeout a member.
    Timeout {
        /// Community name or governance key.
        #[arg(long, short = 'c')]
        community: String,
        /// Member pseudonym or display name.
        #[arg(long, short = 'M')]
        member: String,
        /// Duration (e.g., "5m", "1h", "1d").
        #[arg(long, short = 'd')]
        duration: String,
        /// Reason for the timeout.
        #[arg(long)]
        reason: Option<String>,
    },

    /// List active bans.
    Bans {
        /// Community name or governance key.
        #[arg(long, short = 'c')]
        community: String,
    },
}
