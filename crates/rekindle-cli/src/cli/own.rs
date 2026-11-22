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
    /// Change our avatar (a PNG, JPEG or GIF; it becomes a 128 px WebP).
    Avatar {
        /// The image file; its extension (.png, .jpg, .jpeg, .gif) declares
        /// its kind.
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
    /// The games hidden from everyone.
    Hidden,
    /// Hide a game from everyone (still recorded locally).
    Hide {
        /// The game's id in the game database.
        id: u32,
    },
    /// Take a game off the hide list.
    Unhide {
        /// The game's id in the game database.
        id: u32,
    },
    /// Our favourite servers, each with whether an invite may name it.
    Favourites,
    /// Add a favourite server, or relabel one already there.
    FavouriteAdd {
        /// The game's id in the game database.
        id: u32,
        /// The server, host:port.
        #[arg(value_parser = rekindle_types::game_server::ServerAddress::parse)]
        server: rekindle_types::game_server::ServerAddress,
        /// A label of at most 64 characters.
        #[arg(long, default_value = "")]
        label: String,
    },
    /// Remove a favourite server.
    FavouriteRemove {
        /// The game's id in the game database.
        id: u32,
        /// The server, host:port.
        #[arg(value_parser = rekindle_types::game_server::ServerAddress::parse)]
        server: rekindle_types::game_server::ServerAddress,
    },
}

/// The identity's tamper-evident audit log.
#[derive(Subcommand)]
pub enum AuditCmd {
    /// Verify the log end to end.
    Verify,
    /// Write the log's entries to a file (a Protected action: the
    /// daemon asks for the passphrase first).
    Export {
        /// Output file path (written owner-only by the daemon).
        path: std::path::PathBuf,
        /// Only the entries after this cursor.
        #[arg(long, default_value = "0")]
        since: u64,
    },
}
