//! Our own profile, game and audit log.

use std::path::PathBuf;

use clap::Subcommand;

/// Our profile.
#[derive(Subcommand)]
pub enum ProfileCmd {
    /// Change the name we show.
    Nickname {
        /// The new display name.
        name: String,
    },
    /// Change our avatar (any PNG, JPEG, GIF or WebP; it becomes a 128 px WebP).
    Avatar {
        /// The image file.
        file: PathBuf,
    },
}

/// The game we show.
#[derive(Subcommand)]
pub enum GameCmd {
    /// Show a game (until the detector sees another).
    Set {
        /// The game's name.
        name: String,
        /// The game's id in the game database.
        #[arg(long)]
        id: Option<u32>,
        /// The server to join, host:port.
        #[arg(long)]
        server: Option<String>,
    },
    /// Show no game.
    Clear,
}

/// The identity's tamper-evident audit log.
#[derive(Subcommand)]
pub enum AuditCmd {
    /// Verify the log end to end.
    Verify,
    /// Print the log's entries.
    Export {
        /// Only the entries after this cursor.
        #[arg(long, default_value = "0")]
        since: u64,
    },
}
