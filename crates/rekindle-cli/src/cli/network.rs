//! Node lifecycle and network CLI types.

use clap::Subcommand;

/// Transport node lifecycle subcommands.
#[derive(Subcommand)]
pub enum NodeCmd {
    /// Start the Veilid transport node.
    Start {
        /// Max wait for network attachment in seconds.
        #[arg(long, default_value = "30")]
        attach_timeout: u64,

        /// Run in foreground (don't daemonize).
        #[arg(long)]
        foreground: bool,
    },

    /// Graceful shutdown.
    Stop,

    /// Stop + start.
    Restart,

    /// Re-attach after detach.
    Attach,

    /// Detach from network (keep node alive).
    Detach,
}

/// Network status and peer management subcommands.
#[derive(Subcommand)]
pub enum NetworkCmd {
    /// Connection state, route health, peer count.
    Status,

    /// List known peers with circuit breaker state.
    Peers {
        /// Output format override.
        #[arg(long)]
        format: Option<String>,
    },

    /// Show allocated/imported routes.
    Routes {
        /// Force route refresh now.
        #[arg(long)]
        refresh: bool,
    },

    /// Show resolved safety routing config.
    Config,
}
