//! Config, export, and import CLI types.

use std::path::PathBuf;

use clap::Subcommand;

/// Configuration inspection subcommands.
#[derive(Subcommand)]
pub enum ConfigCmd {
    /// Show resolved config with provenance.
    Show,

    /// Show config file search paths.
    Paths,

    /// Validate config files and report errors.
    Validate,
}

/// Data export subcommands.
#[derive(Subcommand)]
pub enum ExportCmd {
    /// Export the unlocked identity as an archive (as `identity export`).
    Identity {
        /// Output file path (written owner-only).
        path: PathBuf,
        /// Read the passphrase from this file (`-` for stdin).
        #[arg(long)]
        passphrase_file: Option<PathBuf>,
    },

    /// Export friend list as JSON.
    Friends {
        /// Output file path.
        path: PathBuf,
    },

    /// Export community membership as JSON.
    Communities {
        /// Output file path.
        path: PathBuf,
    },
}

/// Data import subcommands.
#[derive(Subcommand)]
pub enum ImportCmd {
    /// Import an identity archive (as `identity import`).
    Identity {
        /// Input file path.
        path: PathBuf,
        /// Read the passphrase from this file (`-` for stdin).
        #[arg(long)]
        passphrase_file: Option<PathBuf>,
    },
}
