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
