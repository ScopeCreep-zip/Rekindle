//! `rekindle skin …` (plan E7.7.2): the skin library, the identity's skin
//! and the device login skin.

use std::path::PathBuf;

use clap::Subcommand;

/// Skins.
#[derive(Subcommand)]
pub enum SkinCmd {
    /// The built-in skins and the library, with the skin in use and the
    /// login skin.
    List,
    /// Import a `.rkskin` package into the library (validated first).
    Import {
        /// The package file.
        file: PathBuf,
    },
    /// Use a skin: `builtin:<id>` or `pkg:<hash>`.
    Select {
        /// The skin id.
        id: String,
    },
    /// Remove a library package (`pkg:<hash>`); not the skin in use.
    Remove {
        /// The skin id.
        id: String,
    },
    /// The skin this device's login screen shows, before unlock. A
    /// library skin is copied without its name, author, source or sounds,
    /// and anyone at the screen can see it.
    SetLogin {
        /// The skin id.
        id: String,
    },
    /// Settle a library skin's 15 s trial ("Keep this skin?"): keep it,
    /// or with `--revert` go back to the skin before it now.
    Keep {
        /// The skin on trial (`pkg:<hash>`).
        id: String,
        /// Go back to the previous skin instead of keeping this one.
        #[arg(long)]
        revert: bool,
    },
}
