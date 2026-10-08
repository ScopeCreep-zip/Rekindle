//! Key management CLI types.
//!
//! Community and channel references are `--flag` arguments.

use clap::Subcommand;

/// Cryptographic key management subcommands.
#[derive(Subcommand)]
pub enum KeyCmd {
    /// MEK (Message Encryption Key) operations.
    #[command(subcommand)]
    Mek(MekCmd),

    /// Prekey bundle management.
    #[command(subcommand)]
    Prekeys(PrekeyCmd),

    /// Show crypto state for a community (MEK cache, session count).
    Inspect {
        /// Community name or governance key.
        #[arg(long, short = 'c')]
        community: String,
    },
}

/// MEK subcommands.
#[derive(Subcommand)]
pub enum MekCmd {
    /// List cached MEKs per channel with generation.
    List {
        /// Community name or governance key.
        #[arg(long, short = 'c')]
        community: String,
    },

    /// Force MEK rotation (requires MANAGE_CHANNELS).
    Rotate {
        /// Community name or governance key.
        #[arg(long, short = 'c')]
        community: String,
        /// Channel name or ID.
        #[arg(long, short = 'C')]
        channel: String,
    },

    /// Request current MEK from peers.
    Request {
        /// Community name or governance key.
        #[arg(long, short = 'c')]
        community: String,
        /// Channel name or ID.
        #[arg(long, short = 'C')]
        channel: String,
    },
}

/// Prekey subcommands.
#[derive(Subcommand)]
pub enum PrekeyCmd {
    /// Show prekey availability count.
    Status,

    /// Generate and publish fresh prekeys.
    Replenish,
}
