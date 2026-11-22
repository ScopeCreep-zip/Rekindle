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
