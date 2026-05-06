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
    /// Export identity bundle.
    Identity {
        /// Output file path.
        path: PathBuf,
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
    /// Import identity bundle.
    Identity {
        /// Input file path.
        path: PathBuf,
    },
}

