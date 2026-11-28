//! Parse tests for the top-level command tree: every top-level command,
//! the global flags, and the parse errors.

use clap::Parser;

use super::*;

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
fn parse_init_without_prompts() {
    let cli = Cli::try_parse_from([
        "rekindle",
        "init",
        "--no-input",
        "--display-name",
        "alice",
        "--passphrase-file",
        "-",
    ])
    .unwrap();
    assert!(cli.no_input);
    if let Some(Command::Init(args)) = cli.command {
        assert_eq!(args.display_name.as_deref(), Some("alice"));
        assert_eq!(
            args.passphrase_file.as_deref(),
            Some(std::path::Path::new("-"))
        );
    } else {
        panic!("expected Init command");
    }
}

#[test]
fn parse_unlock_and_lock() {
    let cli = Cli::try_parse_from(["rekindle", "unlock", "--passphrase-file", "pw"]).unwrap();
    assert!(matches!(cli.command, Some(Command::Unlock(_))));
    let cli = Cli::try_parse_from(["rekindle", "lock"]).unwrap();
    assert!(matches!(cli.command, Some(Command::Lock)));
}

#[test]
fn parse_confirm_and_yes() {
    let cli = Cli::try_parse_from([
        "rekindle",
        "community",
        "leave",
        "--community",
        "Team",
        "--confirm=Team",
    ])
    .unwrap();
    assert_eq!(cli.confirm.as_deref(), Some("Team"));
    let cli =
        Cli::try_parse_from(["rekindle", "-y", "friend", "remove", "--friend", "abcd"]).unwrap();
    assert!(cli.yes);
    let cli = Cli::try_parse_from([
        "rekindle",
        "identity",
        "delete",
        "abcd",
        "--passphrase-file",
        "-",
    ])
    .unwrap();
    assert!(matches!(
        cli.command,
        Some(Command::Identity(IdentityCmd::Delete {
            passphrase_file: Some(_),
            ..
        }))
    ));
}

#[test]
fn parse_unlock_identity_and_logout() {
    let cli = Cli::try_parse_from(["rekindle", "unlock", "--identity", "abcd"]).unwrap();
    assert!(matches!(
        cli.command,
        Some(Command::Unlock(UnlockArgs {
            identity: Some(_),
            ..
        }))
    ));
    let cli = Cli::try_parse_from(["rekindle", "logout"]).unwrap();
    assert!(matches!(cli.command, Some(Command::Logout)));
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
    let cli = Cli::try_parse_from(["rekindle", "community", "create", "my community"]).unwrap();
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
fn parse_channel_forum_post() {
    let args = [
        "rekindle",
        "channel",
        "forum-post",
        "-c",
        "Guild",
        "-C",
        "ideas",
        "-t",
        "Share your maps",
        "-m",
        "Here is mine",
    ];
    for tag in [None, Some("maps")] {
        let mut argv = args.to_vec();
        if let Some(tag) = tag {
            argv.extend(["--tag", tag]);
        }
        let cli = Cli::try_parse_from(argv).unwrap();
        let Some(Command::Channel(ChannelCmd::ForumPost {
            community,
            channel,
            title,
            message,
            tag: parsed_tag,
        })) = cli.command
        else {
            panic!("expected a forum post");
        };
        assert_eq!(community, "Guild");
        assert_eq!(channel, "ideas");
        assert_eq!(title, "Share your maps");
        assert_eq!(message, "Here is mine");
        assert_eq!(parsed_tag.as_deref(), tag);
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
    let cli =
        Cli::try_parse_from(["rekindle", "dm", "send", "-f", "alice", "-m", "hey there"]).unwrap();
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
        "rekindle://add/abcdef1234567890",
        "-m",
        "let's connect",
    ])
    .unwrap();
    if let Some(Command::Friend(FriendCmd::Add { code, message })) = cli.command {
        assert_eq!(code, "rekindle://add/abcdef1234567890");
        assert_eq!(message.as_deref(), Some("let's connect"));
    } else {
        panic!("expected Friend Add");
    }
}

#[test]
fn parse_voice_join() {
    let cli = Cli::try_parse_from([
        "rekindle", "voice", "join", "-c", "gaming", "-C", "voice-1", "--muted",
    ])
    .unwrap();
    if let Some(Command::Voice(VoiceCmd::Join {
        community,
        channel,
        muted,
        deafened,
        watch,
    })) = cli.command
    {
        assert_eq!(community, "gaming");
        assert_eq!(channel, "voice-1");
        assert!(muted);
        assert!(!deafened);
        assert!(!watch);
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
    let cli = Cli::try_parse_from([
        "rekindle", "--script", "channel", "watch", "-c", "com", "-C", "ch",
    ])
    .unwrap();
    assert!(cli.script);
}

#[test]
fn parse_community_watch() {
    let cli =
        Cli::try_parse_from(["rekindle", "--script", "community", "watch", "-c", "Dev"]).unwrap();
    assert!(cli.script);
    assert!(matches!(
        cli.command,
        Some(Command::Community(CommunityCmd::Watch { community: Some(ref c) })) if c == "Dev"
    ));
    let all = Cli::try_parse_from(["rekindle", "community", "watch"]).unwrap();
    assert!(matches!(
        all.command,
        Some(Command::Community(CommunityCmd::Watch { community: None }))
    ));
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
    assert!(Cli::try_parse_from(["rekindle", "channel", "send", "-c", "com", "-C", "ch"]).is_err());
}

#[test]
fn parse_network_alias() {
    let cli = Cli::try_parse_from(["rekindle", "net", "status"]).unwrap();
    assert!(matches!(
        cli.command,
        Some(Command::Network(NetworkCmd::Status))
    ));
}

#[test]
fn parse_activity_pages_and_settings_activity_log() {
    let cli = Cli::try_parse_from(["rekindle", "activity"]).unwrap();
    assert!(matches!(
        cli.command,
        Some(Command::Activity(ActivityArgs {
            before: None,
            limit: 20
        }))
    ));
    let cli =
        Cli::try_parse_from(["rekindle", "activity", "--before", "41", "--limit", "5"]).unwrap();
    assert!(matches!(
        cli.command,
        Some(Command::Activity(ActivityArgs {
            before: Some(41),
            limit: 5
        }))
    ));
    let cli = Cli::try_parse_from(["rekindle", "settings", "activity-log", "off"]).unwrap();
    assert!(matches!(
        cli.command,
        Some(Command::Settings(SettingsCmd::ActivityLog {
            state: Switch::Off
        }))
    ));
}

/// Plan E3.4 S6: `community set-name <community> <name>`, `""` clearing it.
#[test]
fn parse_community_set_name() {
    let cli = Cli::try_parse_from(["rekindle", "community", "set-name", "abcd", "Bea"]).unwrap();
    assert!(matches!(
        cli.command,
        Some(Command::Community(crate::cli::CommunityCmd::SetName { ref community, ref name, clear }))
            if community == "abcd" && name.as_deref() == Some("Bea") && !clear
    ));
    let cli =
        Cli::try_parse_from(["rekindle", "community", "set-name", "abcd", "--clear"]).unwrap();
    assert!(matches!(
        cli.command,
        Some(Command::Community(crate::cli::CommunityCmd::SetName {
            name: None,
            clear: true,
            ..
        }))
    ));
}

#[test]
fn parse_community_profile_edits() {
    use rekindle_types::dto::community_profile::ProfileField;

    let cli = Cli::try_parse_from([
        "rekindle",
        "community",
        "profile",
        "-c",
        "abcd",
        "--bio",
        "At the LAN",
        "--clear",
        "banner-ref",
    ])
    .unwrap();
    let Some(Command::Community(crate::cli::CommunityCmd::Profile(args))) = cli.command else {
        panic!("profile command")
    };
    let patch = args.patch().unwrap();
    assert_eq!(patch.bio, ProfileField::Set("At the LAN".into()));
    assert_eq!(patch.banner_ref, ProfileField::Clear);
    assert_eq!(patch.avatar_ref, ProfileField::Unchanged);
}

/// Friend flags take a display name or nickname as given; `friend_ref`
/// turns it into a key against the Host's friend list (live finding AB2).
#[test]
fn parse_friend_flags_by_name() {
    let cli = Cli::try_parse_from([
        "rekindle",
        "group",
        "create",
        "-n",
        "raid",
        "-m",
        "alice",
        "-m",
        "Bob Smith",
        "--from",
        "alice",
    ])
    .unwrap();
    let Some(Command::Group(GroupDmCmd::Create {
        members,
        origin_peer,
        ..
    })) = cli.command
    else {
        panic!("expected group create");
    };
    assert_eq!(members, ["alice", "Bob Smith"]);
    assert_eq!(origin_peer.as_deref(), Some("alice"));
    for args in [
        ["rekindle", "dm", "history", "-f", "alice"],
        ["rekindle", "dm", "read", "-f", "alice"],
        ["rekindle", "friend", "remove", "-f", "alice"],
    ] {
        let cli = Cli::try_parse_from(args).unwrap();
        let Some(
            Command::Dm(DmCmd::History { friend, .. } | DmCmd::Read { friend })
            | Command::Friend(FriendCmd::Remove { friend }),
        ) = cli.command
        else {
            panic!("{args:?}");
        };
        assert_eq!(friend, "alice", "{args:?}");
    }
}

#[test]
fn community_focus_maps_arguments_to_the_host_request() {
    use rekindle_ipc::protocol::{ChannelMessageRequest, CommunityRequest, IpcRequest};
    use rekindle_types::display::CommunityDetail;

    let detail: CommunityDetail = serde_json::from_value(serde_json::json!({
        "anchor": "anchor-key",
        "name": "Dev Team",
        "description": "",
        "creator_pseudonyms": [],
        "created_at": 0,
        "member_count": 1,
        "our_pseudonym": "member-key",
        "our_roles": [],
        "roles": [],
        "channels": [
            { "id": "text-id", "name": "General Chat", "kind": "text", "topic": "",
              "mekGeneration": 0, "sortOrder": 0 },
            { "id": "voice-id", "name": "Stand-up", "kind": "voice", "topic": "",
              "mekGeneration": 0, "sortOrder": 1 }
        ]
    }))
    .unwrap();

    for (tail, expected_channel, expected_kind) in [
        (&["General Chat"][..], Some("text-id"), "text"),
        (&["voice-id", "--voice"][..], Some("voice-id"), "voice"),
        (&[][..], None, "text"),
    ] {
        let args = ["rekindle", "community", "focus", "Dev Team"]
            .into_iter()
            .chain(tail.iter().copied());
        let cli = Cli::try_parse_from(args).unwrap();
        let Some(Command::Community(CommunityCmd::Focus {
            community,
            channel,
            voice,
        })) = cli.command
        else {
            panic!("expected a focus command");
        };
        assert_eq!(community, detail.name);
        let request = crate::community::focus_request(&detail, channel.as_deref(), voice).unwrap();
        let IpcRequest::Community(CommunityRequest::Messages(
            ChannelMessageRequest::ChannelSetActive {
                community,
                channel,
                kind,
            },
        )) = request
        else {
            panic!("expected ChannelSetActive");
        };
        assert_eq!(community, detail.anchor);
        assert_eq!(channel.as_deref(), expected_channel);
        assert_eq!(kind, expected_kind);
    }
}
