//! Identity and status CLI types.

use std::path::PathBuf;

use clap::{Args, Subcommand};

/// Arguments for `rekindle init`.
#[derive(Args)]
pub struct InitArgs {
    /// Display name for your profile.
    #[arg(long)]
    pub display_name: Option<String>,

    /// Storage directory override.
    #[arg(long)]
    pub storage: Option<PathBuf>,

    /// Fail if any prompts would be needed (CI/scripting mode).
    #[arg(long)]
    pub non_interactive: bool,

    /// Also export identity bundle on creation.
    #[arg(long)]
    pub export_identity: Option<PathBuf>,

    /// Factory reset — destroy all local data.
    /// Named to be unmistakable in shell history.
    #[arg(long)]
    pub wipe_all_data: bool,
}

/// Arguments for `rekindle status`.
#[derive(Args)]
pub struct StatusArgs {
    /// Run diagnostic checks (node, crypto, network, storage, all).
    #[arg(long, num_args = 0..=1, default_missing_value = "all")]
    pub doctor: Option<String>,

    /// Doctor output format: text, json.
    #[arg(long, requires = "doctor")]
    pub output: Option<String>,

    /// Set exit code from doctor results (10/11/12).
    #[arg(long, requires = "doctor")]
    pub exit_code: bool,

    /// Suppress doctor output, exit code only.
    #[arg(long)]
    pub quiet: bool,

    /// Continuous status refresh (2s interval).
    #[arg(long)]
    pub watch: bool,
}

/// Identity management subcommands.
#[derive(Subcommand)]
pub enum IdentityCmd {
    /// Show local identity (pubkey, display name, DHT keys).
    Show {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },

    /// Export encrypted identity bundle.
    Export {
        /// Output file path.
        path: PathBuf,
        /// Protect with passphrase (prompted or stdin).
        #[arg(long)]
        passphrase: bool,
    },

    /// Import identity bundle (device migration).
    Import {
        /// Input file path.
        path: PathBuf,
        /// Decrypt with passphrase.
        #[arg(long)]
        passphrase: bool,
    },

    /// Rotate Ed25519 identity keypair.
    Rotate {
        /// Skip confirmation.
        #[arg(long)]
        force: bool,
    },

    /// Destroy local identity (requires typed confirmation).
    Destroy,
}
