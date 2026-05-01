//! Config, export, import, and doctor CLI types.

use std::path::PathBuf;

use clap::{Args, Subcommand};

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

/// Arguments for `rekindle doctor`.
#[derive(Args)]
pub struct DoctorArgs {
    /// Categories to check: node, crypto, network, storage, all.
    #[arg(default_value = "all")]
    pub categories: String,

    /// Output format: text, json.
    #[arg(long, default_value = "text")]
    pub output: String,

    /// Set exit code from doctor results (10=pass, 11=fail, 12=warn).
    #[arg(long)]
    pub exit_code: bool,

    /// Suppress output, exit code only.
    #[arg(long)]
    pub quiet: bool,
}
