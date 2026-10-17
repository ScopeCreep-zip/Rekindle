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

    /// Read the passphrase from this file (`-` for stdin) instead of a
    /// prompt. Never pass a passphrase as a flag value.
    #[arg(long)]
    pub passphrase_file: Option<PathBuf>,

    /// Also export identity bundle on creation.
    #[arg(long)]
    pub export_identity: Option<PathBuf>,

    /// Factory reset — destroy all local data.
    /// Named to be unmistakable in shell history.
    #[arg(long)]
    pub wipe_all_data: bool,
}

/// Arguments for `rekindle unlock`.
#[derive(Args)]
pub struct UnlockArgs {
    /// The identity to unlock (a public key from `rekindle identity list`);
    /// optional when the device holds one identity.
    #[arg(long)]
    pub identity: Option<String>,

    /// Read the passphrase from this file (`-` for stdin) instead of a
    /// prompt. Never pass a passphrase as a flag value.
    #[arg(long)]
    pub passphrase_file: Option<PathBuf>,
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

    /// The identities on this device.
    List,

    /// Export the unlocked identity as an archive sealed by its
    /// passphrase (its vault), for another device.
    Export {
        /// Output file path (written owner-only).
        path: PathBuf,
        /// Read the passphrase from this file (`-` for stdin).
        #[arg(long)]
        passphrase_file: Option<PathBuf>,
    },

    /// Import an identity archive (device migration); it stays locked.
    Import {
        /// Input file path.
        path: PathBuf,
        /// Read the passphrase from this file (`-` for stdin).
        #[arg(long)]
        passphrase_file: Option<PathBuf>,
    },

    /// Rotate the unlocked identity's Ed25519 keypair; the daemon is left
    /// locked, and the identity unlocks under its new key.
    Rotate {
        /// Read the passphrase from this file (`-` for stdin).
        #[arg(long)]
        passphrase_file: Option<PathBuf>,
    },

    /// Delete an identity from this device (the host asks for its
    /// passphrase).
    Delete {
        /// The identity's public key (`rekindle identity list`).
        identity: String,
        /// Read the passphrase from this file (`-` for stdin).
        #[arg(long)]
        passphrase_file: Option<PathBuf>,
    },
}
