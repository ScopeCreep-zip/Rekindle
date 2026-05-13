//! CLI argument definitions — single source of truth for the user-facing interface.
//!
//! Every clap derive type lives under `cli/`. Domain-specific subcommand
//! enums live in submodules. No types are defined inline in this file.

mod channel;
mod community;
mod config;
mod events;
mod identity;
mod keys;
mod network;
mod social;
mod system;

pub use channel::{ChannelCmd, VoiceCmd};
pub use community::{CommunityCmd, InviteCmd, ModerateCmd, RoleCmd};
pub use config::{ConfigCmd, ExportCmd, ImportCmd};
pub use events::SocialCmd;
pub use identity::{IdentityCmd, InitArgs, StatusArgs};
pub use keys::{KeyCmd, MekCmd, PrekeyCmd};
pub use network::{NetworkCmd, NodeCmd};
pub use social::{DmCmd, FriendCmd, PresenceCmd};
pub use system::SystemCmd;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// Rekindle — decentralized encrypted communication.
///
/// A CLI and TUI for communities, direct messaging, voice, and identity
/// management on the Veilid network. Run without arguments to launch the
/// interactive TUI dashboard.
#[derive(Parser)]
#[command(name = "rekindle", about = "Decentralized communication", version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Override config file path.
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,

    /// Output format: text, json, jsonl.
    #[arg(long, global = true)]
    pub format: Option<String>,

    /// Disable color output (also respects NO_COLOR env).
    #[arg(long, global = true)]
    pub no_color: bool,

    /// Suppress informational output, errors only.
    #[arg(long, global = true)]
    pub quiet: bool,

    /// Force JSONL output on streaming commands.
    #[arg(long, global = true)]
    pub script: bool,

    /// Increase log verbosity (repeatable: -v, -vv, -vvv).
    #[arg(long, short, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// Override Veilid storage directory.
    #[arg(long, global = true)]
    pub storage: Option<PathBuf>,
}

/// Top-level command dispatch enum.
#[derive(Subcommand)]
pub enum Command {
    /// First-run identity ceremony.
    Init(InitArgs),
    /// Show node status, identity, connectivity.
    Status(StatusArgs),
    /// Identity management.
    #[command(subcommand)]
    Identity(IdentityCmd),
    /// Transport node lifecycle.
    #[command(subcommand)]
    Node(NodeCmd),
    /// Network status and peer management.
    #[command(subcommand, alias = "net")]
    Network(NetworkCmd),
    /// Friend management.
    #[command(subcommand)]
    Friend(FriendCmd),
    /// Direct messaging.
    #[command(subcommand)]
    Dm(DmCmd),
    /// Community lifecycle.
    #[command(subcommand)]
    Community(CommunityCmd),
    /// Role management (requires MANAGE_ROLES).
    #[command(subcommand)]
    Role(RoleCmd),
    /// Moderation (requires KICK/BAN/MODERATE_MEMBERS).
    #[command(subcommand)]
    Moderate(ModerateCmd),
    /// Channel operations.
    #[command(subcommand)]
    Channel(ChannelCmd),
    /// Voice channel operations.
    #[command(subcommand)]
    Voice(VoiceCmd),
    /// Cryptographic key management.
    #[command(subcommand)]
    Key(KeyCmd),
    /// Presence management.
    #[command(subcommand)]
    Presence(PresenceCmd),
    /// Social features (reactions, events, threads, game servers).
    #[command(subcommand)]
    Social(SocialCmd),
    /// System/operator commands (announcements, lockdown, raid alerts).
    #[command(subcommand)]
    System(SystemCmd),
    /// Bulk transfer management (send, receive, status, cancel).
    #[command(subcommand)]
    Transfer(crate::v2::commands::transfer::TransferCmd),
    /// Configuration inspection.
    #[command(subcommand)]
    Config(ConfigCmd),
    /// Export data for backup/migration.
    #[command(subcommand)]
    Export(ExportCmd),
    /// Import data.
    #[command(subcommand)]
    Import(ImportCmd),
    /// Fuzzy file search across the project (powered by fff).
    Search {
        /// Search query (fuzzy matched against file paths).
        query: String,
        /// Maximum results to return.
        #[arg(long, default_value = "20")]
        limit: usize,
    },
    /// Content grep across project files (powered by fff).
    Grep {
        /// Search pattern (literal text or regex).
        query: String,
        /// Use regex mode.
        #[arg(long)]
        regex: bool,
        /// Maximum matches to return.
        #[arg(long, default_value = "50")]
        limit: usize,
        /// Context lines before each match.
        #[arg(long, short = 'B', default_value = "0")]
        before: usize,
        /// Context lines after each match.
        #[arg(long, short = 'A', default_value = "0")]
        after: usize,
    },
    /// Generate a patch from local git changes and send it to a channel or DM.
    ///
    /// Without --send, prints the patch to stdout. With --send, sends it as
    /// a patch message to the specified channel or DM peer.
    Patch {
        /// Specific file paths to include (empty = all changes).
        #[arg(trailing_var_arg = true)]
        files: Vec<String>,
        /// Only include staged changes (git index).
        #[arg(long)]
        staged: bool,
        /// Send the patch to a community channel.
        #[arg(long, short = 'c')]
        channel_community: Option<String>,
        /// Channel name within the community.
        #[arg(long, short = 'C')]
        channel_name: Option<String>,
        /// Send the patch as a DM to a peer.
        #[arg(long, short = 'f')]
        dm_peer: Option<String>,
        /// Description/commit message for the patch.
        #[arg(long, short = 'm')]
        message: Option<String>,
    },
    /// Apply a received patch to the local working tree.
    PatchApply {
        /// Path to a .patch or .diff file to apply.
        path: std::path::PathBuf,
        /// Dry-run: check if patch applies cleanly without modifying files.
        #[arg(long)]
        check: bool,
    },
    /// Generate shell completions.
    Completions {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
}

/// Print shell completions to stdout.
pub fn print_completions(shell: clap_complete::Shell) {
    use clap::CommandFactory;
    let mut cmd = Cli::command();
    clap_complete::generate(shell, &mut cmd, "rekindle", &mut std::io::stdout());
}
