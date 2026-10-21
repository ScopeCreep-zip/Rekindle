//! Our own profile, game, settings and audit log.

use std::path::PathBuf;

use clap::{Subcommand, ValueEnum};

/// Our profile.
#[derive(Subcommand)]
pub enum ProfileCmd {
    /// Change the name we show.
    Nickname {
        /// The new display name.
        name: String,
    },
    /// Change our avatar (a PNG, JPEG or GIF; it becomes a 128 px WebP).
    Avatar {
        /// The image file; its extension (.png, .jpg, .jpeg, .gif) declares
        /// its kind.
        file: PathBuf,
    },
}

/// On or off.
#[derive(Clone, Copy, ValueEnum)]
pub enum Switch {
    On,
    Off,
}

/// The identity's settings.
#[derive(Subcommand)]
pub enum SettingsCmd {
    /// DM read receipts: friends see what we read only when both sides turn
    /// them on, and never while we are Invisible (off by default).
    ReadReceipts {
        /// `on` or `off`.
        state: Switch,
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
