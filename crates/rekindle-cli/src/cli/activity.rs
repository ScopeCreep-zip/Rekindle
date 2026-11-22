//! The activity feed (plan E7.15, the Infoview): one page of entries.

use clap::Args;

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

/// A subject's Infoview (plan E7.15, IV4): our own page, a friend's or a
/// community's.
#[derive(Args)]
pub struct InfoviewArgs {
    /// A friend's page, by public key (hex).
    #[arg(long, conflicts_with = "community")]
    pub friend: Option<String>,
    /// A community's page, by anchor or name.
    #[arg(long)]
    pub community: Option<String>,
}

impl InfoviewArgs {
    /// The subject the flags name; none is our own page.
    #[must_use]
    pub fn subject(&self) -> rekindle_ipc::protocol::InfoviewSubject {
        use rekindle_ipc::protocol::InfoviewSubject;
        match (&self.friend, &self.community) {
            (Some(public_key), _) => InfoviewSubject::Friend {
                public_key: public_key.clone(),
            },
            (None, Some(community)) => InfoviewSubject::Community {
                community: community.clone(),
            },
            (None, None) => InfoviewSubject::Own,
        }
    }
}
