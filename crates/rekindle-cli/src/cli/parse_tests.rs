//! Parse tests for subcommand flags that once shadowed a global flag
//! (live A/B finding 9): a local `verbose: bool` beside the global
//! `verbose: u8` count made `community info` panic on every call, and the
//! local `--format`, `--quiet` and `--storage` copies shadowed the globals.

use clap::{CommandFactory, Parser};

use super::{Cli, Command, CommunityCmd, InviteCmd};

fn parse(args: &[&str]) -> Cli {
    let mut argv = vec!["rekindle"];
    argv.extend_from_slice(args);
    Cli::try_parse_from(argv).unwrap()
}

#[test]
fn the_command_tree_is_well_formed() {
    Cli::command().debug_assert();
}

#[test]
fn community_info_parses_without_and_with_full_and_verbose() {
    let cli = parse(&["community", "info", "-c", "X"]);
    assert_eq!(cli.verbose, 0);
    assert!(matches!(
        cli.command,
        Some(Command::Community(CommunityCmd::Info { ref community, full: false })) if community == "X"
    ));
    let cli = parse(&["community", "info", "-c", "X", "--full", "-v"]);
    assert_eq!(cli.verbose, 1);
    assert!(matches!(
        cli.command,
        Some(Command::Community(CommunityCmd::Info { full: true, .. }))
    ));
}

#[test]
fn format_after_a_listing_subcommand_is_the_global_format() {
    for args in [
        &["community", "list", "--format", "json"][..],
        &["channel", "list", "-c", "X", "--format", "json"],
        &["friend", "list", "--format", "json"],
        &["network", "peers", "--format", "json"],
    ] {
        assert_eq!(parse(args).format.as_deref(), Some("json"), "{args:?}");
    }
}

#[test]
fn quiet_on_a_doctor_run_is_the_global_quiet() {
    assert!(parse(&["status", "--doctor=local", "--quiet"]).quiet);
}

#[test]
fn storage_on_init_is_the_global_storage() {
    let cli = parse(&["init", "--storage", "/tmp/rk"]);
    assert_eq!(
        cli.storage.as_deref(),
        Some(std::path::Path::new("/tmp/rk"))
    );
}

#[test]
fn an_invite_without_max_uses_is_single_use() {
    let cli =
        Cli::try_parse_from(["rekindle", "community", "invite", "create", "-c", "X"]).unwrap();
    assert!(matches!(
        cli.command,
        Some(Command::Community(CommunityCmd::Invite(
            InviteCmd::Create { max_uses: 1, .. }
        )))
    ));
}

#[test]
fn invite_max_uses_is_refused_outside_one_to_a_hundred() {
    let invite = |n: &str| {
        Cli::try_parse_from([
            "rekindle",
            "community",
            "invite",
            "create",
            "-c",
            "X",
            "--max-uses",
            n,
        ])
    };
    assert!(invite("0").is_err());
    assert!(invite("101").is_err());
    for n in ["1", "100"] {
        let cli = invite(n).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Community(CommunityCmd::Invite(InviteCmd::Create { max_uses: m, .. })))
                if m.to_string() == n
        ));
    }
}

#[test]
fn skin_subcommands_parse_to_their_requests() {
    use rekindle_ipc::protocol::IpcRequest;

    use super::SkinCmd;

    let request = |args: &[&str]| match parse(args).command {
        Some(Command::Skin(cmd)) => crate::skin::request(&cmd).unwrap(),
        _ => panic!("{args:?} is not a skin command"),
    };
    assert!(matches!(request(&["skin", "list"]), IpcRequest::SkinList));
    assert!(matches!(
        request(&["skin", "select", "builtin:symbiosis"]),
        IpcRequest::SkinSelect { ref id } if id == "builtin:symbiosis"
    ));
    assert!(matches!(
        request(&["skin", "remove", "pkg:ab"]),
        IpcRequest::SkinRemove { ref id } if id == "pkg:ab"
    ));
    assert!(matches!(
        request(&["skin", "set-login", "pkg:ab"]),
        IpcRequest::SkinSetLogin { ref id } if id == "pkg:ab"
    ));
    assert!(matches!(
        request(&["skin", "keep", "pkg:ab"]),
        IpcRequest::SkinKeep { ref id, keep: true } if id == "pkg:ab"
    ));
    assert!(matches!(
        request(&["skin", "keep", "pkg:ab", "--revert"]),
        IpcRequest::SkinKeep { ref id, keep: false } if id == "pkg:ab"
    ));
    let file =
        std::env::temp_dir().join(format!("rekindle-cli-skin-{}.rkskin", std::process::id()));
    std::fs::write(&file, b"pk").unwrap();
    let imported = request(&["skin", "import", file.to_str().unwrap()]);
    std::fs::remove_file(&file).unwrap();
    assert!(matches!(imported, IpcRequest::SkinImport { ref bytes } if bytes == b"pk"));
    let Some(Command::Skin(missing @ SkinCmd::Import { .. })) =
        parse(&["skin", "import", "/nope/x.rkskin"]).command
    else {
        panic!("skin import parses");
    };
    assert!(
        crate::skin::request(&missing).is_err(),
        "an unreadable file is an error"
    );
    assert!(
        Cli::try_parse_from(["rekindle", "skin", "select"]).is_err(),
        "an id is required"
    );
}

#[test]
fn favourite_server_subcommands_parse_to_their_requests() {
    use rekindle_ipc::protocol::IpcRequest;

    let request = |args: &[&str]| match parse(args).command {
        Some(Command::Game(cmd)) => crate::presence::game_request(&cmd),
        _ => panic!("{args:?} is not a game command"),
    };
    assert!(matches!(
        request(&["game", "favourites"]),
        IpcRequest::FavouriteServersGet
    ));
    assert!(matches!(
        request(&["game", "servers"]),
        IpcRequest::ServersView
    ));
    assert!(matches!(
        request(&["game", "recent-remove", "4181", "203.0.113.7:27015"]),
        IpcRequest::RecentServerRemove { game_id: 4181, ref server }
            if server.as_str() == "203.0.113.7:27015"
    ));
    assert!(matches!(
        request(&["game", "favourite-add", "4181", "203.0.113.7:27015", "--label", "Scrims"]),
        IpcRequest::FavouriteServerAdd { game_id: 4181, ref server, ref label }
            if server.as_str() == "203.0.113.7:27015" && label == "Scrims"
    ));
    assert!(matches!(
        request(&["game", "favourite-remove", "4181", "203.0.113.7:27015"]),
        IpcRequest::FavouriteServerRemove { game_id: 4181, ref server }
            if server.as_str() == "203.0.113.7:27015"
    ));
    assert!(
        Cli::try_parse_from(["rekindle", "game", "favourite-add", "4181", "host/path:1"]).is_err(),
        "an unparsed address never leaves"
    );
}

#[test]
fn game_invite_parses_to_its_request() {
    use rekindle_ipc::protocol::IpcRequest;

    let args = [
        "game",
        "invite",
        "--friend",
        "ab12",
        "--id",
        "4181",
        "--server",
        "203.0.113.7:27015",
    ];
    let request = match parse(&args).command {
        Some(Command::Game(cmd)) => crate::presence::game_request(&cmd),
        _ => panic!("{args:?} is not a game command"),
    };
    assert!(matches!(
        request,
        IpcRequest::GameInviteSend { ref public_key, game_id: 4181, ref server }
            if public_key == "ab12" && server.as_str() == "203.0.113.7:27015"
    ));
    assert!(
        Cli::try_parse_from([
            "rekindle",
            "game",
            "invite",
            "--friend",
            "ab12",
            "--id",
            "4181",
            "--server",
            "host/path:1",
        ])
        .is_err(),
        "an unparsed address never leaves"
    );
    assert!(
        Cli::try_parse_from(["rekindle", "game", "invite", "--friend", "ab12", "--id", "4181"])
            .is_err(),
        "the server is required"
    );
}

#[test]
fn transfer_subcommands_parse_to_their_requests() {
    use rekindle_ipc::protocol::{IpcRequest, TransferRequest as T};

    let id = "0123456789abcdef0123456789abcdef";
    let request = |args: &[&str]| match parse(args).command {
        Some(Command::Transfer(cmd)) => crate::transfer::request(&cmd).unwrap(),
        _ => panic!("{args:?} is not a transfer command"),
    };
    assert!(matches!(
        request(&["transfer", "list"]),
        IpcRequest::Transfer(T::List)
    ));
    assert!(matches!(
        request(&["transfer", "start", id, "--community", "VLD0:c", "--channel", "ch", "--dest", "/tmp/f"]),
        IpcRequest::Transfer(T::Start { ref attachment_id, dest: Some(ref d), .. })
            if attachment_id == id && d == "/tmp/f"
    ));
    assert!(matches!(
        request(&["transfer", "stop", id]),
        IpcRequest::Transfer(T::Stop { ref attachment_id }) if attachment_id == id
    ));
    assert!(matches!(
        request(&["transfer", "open", id]),
        IpcRequest::Transfer(T::Open { .. })
    ));
    assert!(matches!(
        request(&["transfer", "remove", id, "--delete-file"]),
        IpcRequest::Transfer(T::Remove {
            delete_file: true,
            ..
        })
    ));
}
