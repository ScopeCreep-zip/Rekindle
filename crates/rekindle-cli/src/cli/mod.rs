//! CLI argument definitions — single source of truth for the user-facing interface.
//!
//! Every clap derive type lives under `cli/`. The top-level `Cli` struct and
//! `Command` enum are here; domain-specific subcommand enums live in submodules.
//! No clap types exist outside this module hierarchy.

mod channel;
mod community;
mod config;
mod identity;
mod keys;
mod network;
mod social;

// Re-export all types so callers use `cli::Cli`, `cli::Command`, etc.
pub use channel::{ChannelCmd, VoiceCmd};
pub use community::{CommunityCmd, InviteCmd, ModerateCmd, RoleCmd};
pub use config::{ConfigCmd, ExportCmd, ImportCmd};
pub use identity::{IdentityCmd, InitArgs, StatusArgs};
pub use keys::{KeyCmd, MekCmd, PrekeyCmd};
pub use network::{NetworkCmd, NodeCmd};
pub use social::{DmCmd, FriendCmd, PresenceCmd};

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

    /// Force JSONL output on streaming commands (watch, voice join).
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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    // ── Argument parsing: every top-level command parses ────────────

    #[test]
    fn parse_bare_invocation() {
        let cli = Cli::try_parse_from(["rekindle"]).unwrap();
        assert!(cli.command.is_none());
    }

    #[test]
    fn parse_status() {
        let cli = Cli::try_parse_from(["rekindle", "status"]).unwrap();
        assert!(matches!(cli.command, Some(Command::Status(_))));
    }

    #[test]
    fn parse_init() {
        let cli = Cli::try_parse_from(["rekindle", "init"]).unwrap();
        assert!(matches!(cli.command, Some(Command::Init(_))));
    }

    #[test]
    fn parse_init_non_interactive() {
        let cli = Cli::try_parse_from([
            "rekindle",
            "init",
            "--non-interactive",
            "--display-name",
            "alice",
        ])
        .unwrap();
        if let Some(Command::Init(args)) = cli.command {
            assert!(args.non_interactive);
            assert_eq!(args.display_name.as_deref(), Some("alice"));
        } else {
            panic!("expected Init command");
        }
    }

    #[test]
    fn parse_status_doctor() {
        let cli = Cli::try_parse_from(["rekindle", "status", "--doctor"]).unwrap();
        if let Some(Command::Status(args)) = cli.command {
            assert_eq!(args.doctor.as_deref(), Some("all"));
        } else {
            panic!("expected Status");
        }
    }

    #[test]
    fn parse_status_watch() {
        let cli = Cli::try_parse_from(["rekindle", "status", "--watch"]).unwrap();
        if let Some(Command::Status(args)) = cli.command {
            assert!(args.watch);
        } else {
            panic!("expected Status");
        }
    }

    #[test]
    fn parse_status_watch_doctor() {
        let cli = Cli::try_parse_from(["rekindle", "status", "--watch", "--doctor"]).unwrap();
        if let Some(Command::Status(args)) = cli.command {
            assert!(args.watch);
            assert_eq!(args.doctor.as_deref(), Some("all"));
        } else {
            panic!("expected Status");
        }
    }

    #[test]
    fn parse_identity_show() {
        let cli = Cli::try_parse_from(["rekindle", "identity", "show"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Identity(IdentityCmd::Show { json: false }))
        ));
    }

    #[test]
    fn parse_identity_show_json() {
        let cli = Cli::try_parse_from(["rekindle", "identity", "show", "--json"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Identity(IdentityCmd::Show { json: true }))
        ));
    }

    #[test]
    fn parse_community_create() {
        let cli = Cli::try_parse_from([
            "rekindle",
            "community",
            "create",
            "--name",
            "my community",
        ])
        .unwrap();
        if let Some(Command::Community(CommunityCmd::Create { name, .. })) = cli.command {
            assert_eq!(name, "my community");
        } else {
            panic!("expected Community Create");
        }
    }

    #[test]
    fn parse_channel_send() {
        let cli = Cli::try_parse_from([
            "rekindle",
            "channel",
            "send",
            "-c",
            "dev-team",
            "-C",
            "general",
            "-m",
            "hello world",
        ])
        .unwrap();
        if let Some(Command::Channel(ChannelCmd::Send {
            community,
            channel,
            message,
            ..
        })) = cli.command
        {
            assert_eq!(community, "dev-team");
            assert_eq!(channel, "general");
            assert_eq!(message, "hello world");
        } else {
            panic!("expected Channel Send");
        }
    }

    #[test]
    fn parse_channel_send_with_spaces() {
        let cli = Cli::try_parse_from([
            "rekindle",
            "channel",
            "send",
            "-c",
            "my big community",
            "-C",
            "channel with spaces",
            "-m",
            "message with \"quotes\" and spaces",
        ])
        .unwrap();
        if let Some(Command::Channel(ChannelCmd::Send {
            community,
            channel,
            message,
            ..
        })) = cli.command
        {
            assert_eq!(community, "my big community");
            assert_eq!(channel, "channel with spaces");
            assert!(message.contains("quotes"));
        } else {
            panic!("expected Channel Send");
        }
    }

    #[test]
    fn parse_dm_send() {
        let cli = Cli::try_parse_from([
            "rekindle",
            "dm",
            "send",
            "-f",
            "alice",
            "-m",
            "hey there",
        ])
        .unwrap();
        if let Some(Command::Dm(DmCmd::Send {
            friend, message, ..
        })) = cli.command
        {
            assert_eq!(friend, "alice");
            assert_eq!(message, "hey there");
        } else {
            panic!("expected Dm Send");
        }
    }

    #[test]
    fn parse_friend_add() {
        let cli = Cli::try_parse_from([
            "rekindle",
            "friend",
            "add",
            "-t",
            "abcdef1234567890",
            "-m",
            "let's connect",
        ])
        .unwrap();
        if let Some(Command::Friend(FriendCmd::Add { target, message })) = cli.command {
            assert_eq!(target, "abcdef1234567890");
            assert_eq!(message.as_deref(), Some("let's connect"));
        } else {
            panic!("expected Friend Add");
        }
    }

    #[test]
    fn parse_voice_join() {
        let cli = Cli::try_parse_from([
            "rekindle",
            "voice",
            "join",
            "-c",
            "gaming",
            "-C",
            "voice-1",
            "--muted",
        ])
        .unwrap();
        if let Some(Command::Voice(VoiceCmd::Join {
            community,
            channel,
            muted,
            deafened,
        })) = cli.command
        {
            assert_eq!(community, "gaming");
            assert_eq!(channel, "voice-1");
            assert!(muted);
            assert!(!deafened);
        } else {
            panic!("expected Voice Join");
        }
    }

    #[test]
    fn parse_config_show() {
        let cli = Cli::try_parse_from(["rekindle", "config", "show"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Config(ConfigCmd::Show))
        ));
    }

    #[test]
    fn parse_completions() {
        let cli = Cli::try_parse_from(["rekindle", "completions", "bash"]).unwrap();
        assert!(matches!(cli.command, Some(Command::Completions { .. })));
    }

    // ── Global flags ───────────────────────────────────────────────

    #[test]
    fn parse_global_format_json() {
        let cli = Cli::try_parse_from(["rekindle", "--format", "json", "status"]).unwrap();
        assert_eq!(cli.format.as_deref(), Some("json"));
    }

    #[test]
    fn parse_global_no_color() {
        let cli = Cli::try_parse_from(["rekindle", "--no-color", "status"]).unwrap();
        assert!(cli.no_color);
    }

    #[test]
    fn parse_global_verbose() {
        let cli = Cli::try_parse_from(["rekindle", "-vvv", "status"]).unwrap();
        assert_eq!(cli.verbose, 3);
    }

    #[test]
    fn parse_global_quiet() {
        let cli = Cli::try_parse_from(["rekindle", "--quiet", "status"]).unwrap();
        assert!(cli.quiet);
    }

    #[test]
    fn parse_global_script() {
        let cli = Cli::try_parse_from(["rekindle", "--script", "channel", "watch", "-c", "com", "-C", "ch"]).unwrap();
        assert!(cli.script);
    }

    // ── Error cases ────────────────────────────────────────────────

    #[test]
    fn parse_unknown_command_fails() {
        assert!(Cli::try_parse_from(["rekindle", "nonexistent"]).is_err());
    }

    #[test]
    fn parse_channel_send_missing_required_flags() {
        // -c and -C and -m are all required
        assert!(Cli::try_parse_from(["rekindle", "channel", "send"]).is_err());
        assert!(Cli::try_parse_from(["rekindle", "channel", "send", "-c", "com"]).is_err());
        assert!(Cli::try_parse_from([
            "rekindle",
            "channel",
            "send",
            "-c",
            "com",
            "-C",
            "ch"
        ])
        .is_err());
    }

    #[test]
    fn parse_network_alias() {
        let cli = Cli::try_parse_from(["rekindle", "net", "status"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Network(NetworkCmd::Status))
        ));
    }
}
