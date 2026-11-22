//! The activity feed (plan E7.15, the Infoview): one page of entries.

use clap::{Args, Subcommand};

/// One page of the activity feed, newest first.
#[derive(Args)]
pub struct ActivityArgs {
    /// Page on from the last id a page printed.
    #[arg(long)]
    pub before: Option<i64>,
    /// How many entries, 1–100.
    #[arg(long, default_value_t = 20)]
    pub limit: u16,
}

/// A subject's Infoview (plan E7.15, IV4, IV7): our own page (no subject),
/// a friend's, a friend group's or a community's. The Host resolves and
/// refuses subjects; this only names one.
#[derive(Subcommand)]
pub enum InfoviewCmd {
    /// Our own page: the feed, our communities, friends in game.
    Own,
    /// A friend's page, by public key (hex). A display name is not a
    /// subject: names are not unique and a peer chooses its own, so the
    /// Host refuses one (threat model: spoofable display fields).
    Friend {
        /// The friend's public key (hex).
        key: String,
    },
    /// A friend group's page: its id (`rekindle friend group list`), or
    /// `default` for friends in no group.
    Group {
        /// The group id, or `default`.
        #[arg(value_parser = parse_group)]
        group: GroupArg,
    },
    /// A joined community's page, by anchor or name.
    Community {
        /// The community's anchor or name.
        community: String,
    },
}

/// A friend group named on the command line: an id, or the default group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupArg(pub Option<i64>);

fn parse_group(s: &str) -> Result<GroupArg, String> {
    if s == "default" {
        return Ok(GroupArg(None));
    }
    s.parse::<i64>()
        .map(|id| GroupArg(Some(id)))
        .map_err(|_| format!("`{s}` is neither a group id nor `default`"))
}

impl InfoviewCmd {
    /// The subject the command names; no subcommand is our own page.
    #[must_use]
    pub fn subject(cmd: Option<&Self>) -> rekindle_ipc::protocol::InfoviewSubject {
        use rekindle_ipc::protocol::InfoviewSubject;
        match cmd {
            None | Some(Self::Own) => InfoviewSubject::Own,
            Some(Self::Friend { key }) => InfoviewSubject::Friend {
                public_key: key.clone(),
            },
            Some(Self::Group { group }) => InfoviewSubject::FriendGroup { group_id: group.0 },
            Some(Self::Community { community }) => InfoviewSubject::Community {
                community: community.clone(),
            },
        }
    }
}
