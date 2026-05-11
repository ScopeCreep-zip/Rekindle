//! Key management CLI types.

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
    /// Show crypto state for a community.
    Inspect {
        #[arg(long, short = 'c')]
        community: String,
    },
}

/// MEK subcommands.
#[derive(Subcommand)]
pub enum MekCmd {
    /// List cached MEKs per channel with generation.
    List {
        #[arg(long, short = 'c')]
        community: String,
    },
    /// Force MEK rotation (requires MANAGE_CHANNELS).
    Rotate {
        #[arg(long, short = 'c')]
        community: String,
        #[arg(long, short = 'C')]
        channel: String,
    },
    /// Request current MEK from peers.
    Request {
        #[arg(long, short = 'c')]
        community: String,
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
