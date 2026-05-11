//! Community, role, moderation, and invite CLI types.

use std::path::PathBuf;
use clap::Subcommand;

/// Community lifecycle subcommands.
#[derive(Subcommand)]
pub enum CommunityCmd {
    /// Create a new community.
    Create {
        name: String,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        icon: Option<PathBuf>,
    },
    /// Join via invite code or governance key.
    Join {
        #[arg(long, short = 'i')]
        invite: String,
        #[arg(long)]
        display_name: Option<String>,
    },
    /// Leave a community.
    Leave {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long)]
        yes: bool,
    },
    /// List joined communities.
    List {
        #[arg(long)]
        format: Option<String>,
    },
    /// Show community details.
    Info {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long)]
        verbose: bool,
    },
    /// Approve a pending member.
    Approve {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long, short = 'M')]
        member: String,
    },
    /// Reject a pending member.
    Reject {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long, short = 'M')]
        member: String,
        #[arg(long)]
        reason: Option<String>,
    },
    /// List pending join requests.
    Pending {
        #[arg(long, short = 'c')]
        community: String,
    },
    /// Transfer community ownership.
    Transfer {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long, short = 'M')]
        new_owner: String,
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
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long)]
        max_uses: Option<u32>,
        #[arg(long)]
        expires: Option<String>,
    },
    /// List active invites.
    List {
        #[arg(long, short = 'c')]
        community: String,
    },
    /// Revoke an invite.
    Revoke {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long, short = 'i')]
        invite_code: String,
    },
}

/// Role management subcommands.
#[derive(Subcommand)]
pub enum RoleCmd {
    /// List roles with permissions.
    List {
        #[arg(long, short = 'c')]
        community: String,
    },
    /// Create a role.
    Create {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long, short = 'n')]
        name: String,
        #[arg(long)]
        permissions: Option<String>,
        #[arg(long)]
        color: Option<String>,
        #[arg(long)]
        position: Option<u32>,
    },
    /// Update a role.
    Update {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long, short = 'r')]
        role_id: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        permissions: Option<String>,
        #[arg(long)]
        color: Option<String>,
    },
    /// Delete a role.
    Delete {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long, short = 'r')]
        role_id: String,
        #[arg(long)]
        yes: bool,
    },
    /// Assign a role to a member.
    Assign {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long, short = 'M')]
        member: String,
        #[arg(long, short = 'r')]
        role_id: String,
    },
    /// Remove a role from a member.
    Unassign {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long, short = 'M')]
        member: String,
        #[arg(long, short = 'r')]
        role_id: String,
    },
}

/// Moderation subcommands.
#[derive(Subcommand)]
pub enum ModerateCmd {
    /// Kick a member.
    Kick {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long, short = 'M')]
        member: String,
        #[arg(long)]
        reason: Option<String>,
    },
    /// Ban a member.
    Ban {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long, short = 'M')]
        member: String,
        #[arg(long)]
        reason: Option<String>,
    },
    /// Unban a member.
    Unban {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long, short = 'M')]
        member: String,
    },
    /// Timeout a member.
    Timeout {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long, short = 'M')]
        member: String,
        /// Duration (e.g., "5m", "1h", "1d").
        #[arg(long, short = 'd')]
        duration: String,
        #[arg(long)]
        reason: Option<String>,
    },
    /// List active bans.
    Bans {
        #[arg(long, short = 'c')]
        community: String,
    },
}
