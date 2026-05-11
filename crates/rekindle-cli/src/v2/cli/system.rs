//! System/operator CLI types — announcements, lockdown, raid alerts, bootstrap, sync.

use clap::Subcommand;

/// System/operator subcommands.
#[derive(Subcommand)]
pub enum SystemCmd {
    /// Broadcast a system announcement to all community members.
    Announce {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long, short = 'm')]
        body: String,
    },
    /// Toggle raid alert mode.
    RaidAlert {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long)]
        active: bool,
    },
    /// Toggle community lockdown (non-operator send block).
    Lockdown {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long)]
        locked: bool,
    },
    /// Notify a kicked member (point-to-point).
    KickNotify {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long, short = 'M')]
        target: String,
    },
    /// Request bootstrap data from operator (new joiner).
    BootstrapRequest {
        #[arg(long, short = 'c')]
        community: String,
    },
    /// Request channel history sync.
    SyncRequest {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long)]
        channel_id: String,
        #[arg(long)]
        since: u64,
    },
}
