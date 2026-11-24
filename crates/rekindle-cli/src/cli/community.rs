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
    ///
    /// `--confirm` takes the community name you are creating.
    #[command(after_help = "--confirm SUBJECT: the community name, as given.")]
    Create {
        /// Community name.
        name: String,
        /// Community description.
        #[arg(long)]
        description: Option<String>,
        /// Community icon image path.
        #[arg(long)]
        icon: Option<PathBuf>,
        /// Require an approval before a joiner counts as a member.
        ///
        /// Fixed at creation: the merge honours `AdmissionPolicy` only
        /// as the genesis entry, so this cannot be changed later — not
        /// even by an administrator.
        #[arg(long)]
        approval_required: bool,
        /// A friend's identity key to name a co-creator (repeatable): the
        /// genesis waits for each to accept, decline or be removed.
        #[arg(long = "co-creator")]
        co_creators: Vec<String>,
    },

    /// Stop waiting on a co-creator of a pending creation that has not
    /// answered.
    CreatorRemove {
        /// The pending community's anchor key.
        anchor: String,
        /// The co-creator's identity key.
        identity: String,
    },

    /// Cancel a pending creation: its co-creators are told.
    CreateCancel {
        /// The pending community's anchor key.
        anchor: String,
    },

    /// Answer a held co-creator invite.
    CreatorAnswer {
        /// The invite's anchor key.
        anchor: String,
        /// Decline instead of accepting.
        #[arg(long)]
        decline: bool,
    },

    /// Join via invite code or anchor key.
    ///
    /// `--confirm` takes the invite's fingerprint: the 64 hex digits the
    /// review prints (the spaces between groups may be kept), not the
    /// community name, which the joiner does not know yet.
    #[command(after_help = "--confirm SUBJECT: the invite's 64-hex fingerprint from the review.")]
    Join {
        /// Invite code or anchor key.
        #[arg(long, short = 'i')]
        invite: String,
        /// Override display name for this community.
        #[arg(long)]
        display_name: Option<String>,
    },

    /// Leave a community.
    Leave {
        /// Community name or anchor key.
        #[arg(long, short = 'c')]
        community: String,
    },

    /// Set our name in one community (our card, `M`/0); `""` clears it, and
    /// the community shows our short pseudonym fingerprint instead. Our
    /// identity name is never shown to a community.
    SetName {
        /// The community's anchor key.
        community: String,
        /// The name, at most 64 bytes; `""` clears it.
        name: String,
    },

    /// Forget a community you were removed from: its kept history and keys.
    Forget {
        /// Community name or anchor key.
        #[arg(long, short = 'c')]
        community: String,
    },

    /// List joined communities.
    List,

    /// Show community details (metadata, channels, roles, members).
    Info {
        /// Community name or anchor key.
        #[arg(long, short = 'c')]
        community: String,
        /// Include full channel list and role permissions.
        #[arg(long)]
        full: bool,
    },

    /// Approve a pending member from the waiting room.
    Approve {
        /// Community name or anchor key.
        #[arg(long, short = 'c')]
        community: String,
        /// Member pseudonym to approve.
        #[arg(long, short = 'M')]
        member: String,
    },

    /// Reject a pending member from the waiting room.
    Reject {
        /// Community name or anchor key.
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
        /// Community name or anchor key.
        #[arg(long, short = 'c')]
        community: String,
    },

    /// Re-create the community under a new anchor that names it as its
    /// predecessor, with you as sole creator, and offer it to the members.
    PublishSuccessor {
        /// Community name or anchor key.
        #[arg(long, short = 'c')]
        community: String,
        /// A member (pseudonym hex) the successor continues with a sealed
        /// invite; repeat it. Absent, every current member is continued.
        #[arg(long = "continue")]
        continuing: Vec<String>,
    },

    /// Follow an offered successor: join it, keeping this community.
    FollowSuccessor {
        /// Community name or anchor key.
        #[arg(long, short = 'c')]
        community: String,
        /// The successor's anchor key; your invite is the one sealed to
        /// you in its invites record.
        #[arg(long)]
        new_anchor: String,
    },

    /// List the successors offered for a community.
    Successors {
        /// Community name or anchor key.
        #[arg(long, short = 'c')]
        community: String,
    },

    /// Enrol this device in a community with a fresh device key (a second
    /// device, or after your person re-joined from another device).
    EnrolDevice {
        /// Community name or anchor key.
        #[arg(long, short = 'c')]
        community: String,
    },

    /// Retire one of your own device keys in a community (a lost or
    /// stolen device): every member then refuses its later ops.
    RetireDevice {
        /// Community name or anchor key.
        #[arg(long, short = 'c')]
        community: String,
        /// The device key to retire (hex); not this device's own.
        #[arg(long)]
        device_key: String,
    },

    /// Co-sign a member's fork-answering device key back in.
    ReadmitDevice {
        /// Community name or anchor key.
        #[arg(long, short = 'c')]
        community: String,
        /// The member (pseudonym hex) whose device key it is.
        #[arg(long)]
        member: String,
        /// The device key to readmit (hex).
        #[arg(long)]
        device_key: String,
    },

    /// Offer the community to another member: a transfer they complete by
    /// accepting it (plan E3.2 S7s.4).
    Transfer {
        /// Community name or anchor key.
        #[arg(long, short = 'c')]
        community: String,
        /// The recipient's pseudonym key.
        #[arg(long, short = 'M')]
        new_owner: String,
    },

    /// Withdraw our standing transfer offer.
    WithdrawTransfer {
        /// Community name or anchor key.
        #[arg(long, short = 'c')]
        community: String,
    },

    /// Accept the transfer offered to us, collecting each other creator's
    /// link through the named friends.
    AcceptTransfer {
        /// Community name or anchor key.
        #[arg(long, short = 'c')]
        community: String,
    },

    /// Answer a transfer's link request that asks us, with our creator
    /// link into the successor (lead decision (46)).
    AnswerLink {
        /// Community name or anchor key.
        #[arg(long, short = 'c')]
        community: String,
        /// The link request's op hash, hex.
        #[arg(long)]
        request: String,
    },

    /// Invite management.
    #[command(subcommand)]
    Invite(InviteCmd),

    /// Role management (requires MANAGE_ROLES).
    #[command(subcommand)]
    Role(RoleCmd),

    /// Moderation: `kick`, `ban`, `unban`, `timeout`, `bans` (requires
    /// KICK_MEMBERS, BAN_MEMBERS or MODERATE_MEMBERS).
    #[command(flatten)]
    Moderate(ModerateCmd),

    /// Live-stream membership and governance events (joins, leaves,
    /// kicks and bans with their reason, roles, channels) until Ctrl-C.
    Watch {
        /// Community name or anchor key; every community when omitted.
        #[arg(long, short = 'c')]
        community: Option<String>,
    },
}

/// Invite subcommands.
#[derive(Subcommand)]
pub enum InviteCmd {
    /// Generate invite code/link.
    Create {
        /// Community name or anchor key.
        #[arg(long, short = 'c')]
        community: String,
        /// Maximum number of uses, 1 (the default) to 100 (there is no
        /// unlimited invite).
        #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32)
            .range(1..=i64::from(rekindle_types::invite::MAX_USES_PER_INVITE)))]
        max_uses: u32,
        /// Expiration duration (e.g., "24h", "7d", "never").
        #[arg(long)]
        expires: Option<String>,
    },

    /// List active invites.
    List {
        /// Community name or anchor key.
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
        /// Community name or anchor key.
        #[arg(long, short = 'c')]
        community: String,
    },

    /// Create a role.
    Create {
        /// Community name or anchor key.
        #[arg(long, short = 'c')]
        community: String,
        /// Role name.
        #[arg(long, short = 'n')]
        name: String,
        /// Permission bitmask (decimal or hex with 0x prefix).
        #[arg(long)]
        permissions: String,
        /// Role color (hex, e.g., "FF5733").
        #[arg(long)]
        color: Option<String>,
    },

    /// Edit a role: its name, permissions or color.
    Edit {
        /// Community name or anchor key.
        #[arg(long, short = 'c')]
        community: String,
        /// Role ID (32 hex characters).
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
        /// Community name or anchor key.
        #[arg(long, short = 'c')]
        community: String,
        /// Role ID (32 hex characters).
        #[arg(long, short = 'r')]
        role_id: String,
    },

    /// Assign a role to a member.
    Assign {
        /// Community name or anchor key.
        #[arg(long, short = 'c')]
        community: String,
        /// Member pseudonym or display name.
        #[arg(long, short = 'M')]
        member: String,
        /// Role ID (32 hex characters).
        #[arg(long, short = 'r')]
        role_id: String,
    },

    /// Remove a role from a member.
    Unassign {
        /// Community name or anchor key.
        #[arg(long, short = 'c')]
        community: String,
        /// Member pseudonym or display name.
        #[arg(long, short = 'M')]
        member: String,
        /// Role ID (32 hex characters).
        #[arg(long, short = 'r')]
        role_id: String,
    },
}

/// Moderation subcommands (requires moderation permissions).
#[derive(Subcommand)]
pub enum ModerateCmd {
    /// Kick a member from the community.
    Kick {
        /// Community name or anchor key.
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
        /// Community name or anchor key.
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
        /// Community name or anchor key.
        #[arg(long, short = 'c')]
        community: String,
        /// Member pseudonym or display name.
        #[arg(long, short = 'M')]
        member: String,
    },

    /// Timeout a member.
    Timeout {
        /// Community name or anchor key.
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
        /// Community name or anchor key.
        #[arg(long, short = 'c')]
        community: String,
    },
}
