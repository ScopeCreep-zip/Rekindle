//! CLI argument definitions — single source of truth for the user-facing interface.
//!
//! Every clap derive type lives under `cli/`. The top-level `Cli` struct and
//! `Command` enum are here; domain-specific subcommand enums live in submodules.
//! No clap types exist outside this module hierarchy.

mod activity;
mod channel;
#[cfg(test)]
mod command_tests;
mod community;
mod config;
mod group_dm;
mod identity;
mod keys;
mod network;
mod own;
#[cfg(test)]
mod parse_tests;
mod settings;
mod skin;
mod social;

// Re-export all types so callers use `cli::Cli`, `cli::Command`, etc.
pub use activity::{ActivityArgs, InfoviewArgs};
pub use channel::{ChannelCmd, VoiceCmd};
pub use community::{CommunityCmd, InviteCmd, ModerateCmd, RoleCmd};
pub use config::{ConfigCmd, ExportCmd, ImportCmd};
pub use group_dm::GroupDmCmd;
pub use identity::{IdentityCmd, InitArgs, StatusArgs, UnlockArgs};
pub use keys::{KeyCmd, MekCmd, PrekeyCmd};
pub use network::{NetworkCmd, NodeCmd};
pub use own::{AuditCmd, GameCmd, ProfileCmd};
pub use settings::{SettingsCmd, Switch};
pub use skin::SkinCmd;
pub use social::{
    DmCmd, FriendCmd, FriendCodeCmd, FriendInviteCmd, GroupCmd, PresenceCmd, PresenceStatus,
};

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

    /// Force JSONL output on streaming commands (`dm watch`, `channel
    /// watch`, `community watch`, `voice join --watch`).
    /// Enables external accessibility wrappers and scripting pipelines
    /// to consume real-time events without entering the TUI.
    #[arg(long, global = true)]
    pub script: bool,

    /// Increase log verbosity (repeatable: -v, -vv, -vvv).
    #[arg(long, short, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// Override Veilid storage directory.
    #[arg(long, global = true)]
    pub storage: Option<PathBuf>,

    /// Never prompt; fail instead and name the flag that supplies the
    /// input (scripts, CI).
    #[arg(long, global = true)]
    pub no_input: bool,

    /// Commit a consequential action without the y/N prompt (plan D3.4).
    /// It never confirms a trust-changing, destructive or protected one.
    #[arg(long, short = 'y', global = true)]
    pub yes: bool,

    /// Confirm a trust-changing or destructive action by naming its
    /// subject (the community, channel, role or person the review names).
    #[arg(long, global = true, value_name = "SUBJECT")]
    pub confirm: Option<String>,
}

/// Top-level command dispatch enum.
///
/// Every user-facing command is a variant here. Domain-specific subcommands
/// delegate to their own enums in submodules.
#[derive(Subcommand)]
pub enum Command {
    /// First-run identity ceremony.
    Init(InitArgs),

    /// Show node status, identity, connectivity.
    Status(StatusArgs),

    /// Unlock the daemon: load the identity and go online.
    Unlock(UnlockArgs),

    /// Lock the daemon: go offline and drop the identity's keys from memory.
    Lock,

    /// Log out: go offline and close the identity (the daemon keeps no
    /// selection, so the next `unlock` names one).
    Logout,

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

    /// Group DMs (3-8 members).
    #[command(subcommand)]
    Group(GroupDmCmd),

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

    /// Our profile: name and avatar.
    #[command(subcommand)]
    Profile(ProfileCmd),

    /// The identity's settings: read receipts.
    #[command(subcommand)]
    Settings(SettingsCmd),

    /// Skins: the library, the skin in use and the login skin.
    #[command(subcommand)]
    Skin(SkinCmd),

    /// The game we show.
    #[command(subcommand)]
    Game(GameCmd),

    /// The activity feed, newest first.
    Activity(ActivityArgs),
    /// A subject's Infoview: our own page, or a friend's or community's.
    Infoview(InfoviewArgs),

    /// The tamper-evident audit log.
    #[command(subcommand)]
    Audit(AuditCmd),

    /// Take back an action while its offer lasts (the id an answer prints).
    Undo {
        /// The undo offer's id.
        id: String,
    },

    /// Configuration inspection.
    #[command(subcommand)]
    Config(ConfigCmd),

    /// Export data for backup/migration.
    #[command(subcommand)]
    Export(ExportCmd),

    /// Import data.
    #[command(subcommand)]
    Import(ImportCmd),

    /// Generate shell completions.
    Completions {
        /// Shell to generate completions for.
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
