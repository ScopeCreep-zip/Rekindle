use clap::Parser;
use rekindle_ipc::protocol::{ChannelRequest, CommunityRequest, IpcRequest};

use super::request;
use crate::cli::{Cli, Command, CommunityChannelCmd, CommunityCmd, OverwriteCmd};

const ROLE: &str = "0102030405060708090a0b0c0d0e0f10";

fn overwrite(args: &[&str]) -> OverwriteCmd {
    let argv = ["rekindle", "community", "channel", "overwrite"]
        .into_iter()
        .chain(args.iter().copied());
    match Cli::try_parse_from(argv).expect("parses").command {
        Some(Command::Community(CommunityCmd::Channel(CommunityChannelCmd::Overwrite(cmd)))) => cmd,
        _ => panic!("not an overwrite command"),
    }
}

fn channel_request(cmd: &OverwriteCmd) -> ChannelRequest {
    match request(cmd).expect("a request") {
        IpcRequest::Community(CommunityRequest::Channels(req)) => req,
        _ => panic!("not a channel request"),
    }
}

#[test]
fn set_ors_every_allow_and_deny_and_names_a_role() {
    let cmd = overwrite(&[
        "set",
        "-c",
        "anchor",
        "-C",
        "chan",
        "--target",
        &format!("role:{ROLE}"),
        "--allow",
        "1",
        "--allow",
        "0x4",
        "--deny",
        "0x800",
    ]);
    match channel_request(&cmd) {
        ChannelRequest::ChannelOverwriteSet {
            community,
            channel_id,
            target_type,
            target_id,
            allow,
            deny,
        } => {
            assert_eq!(
                (community.as_str(), channel_id.as_str()),
                ("anchor", "chan")
            );
            assert_eq!((target_type.as_str(), target_id.as_str()), ("role", ROLE));
            assert_eq!((allow, deny), (5, 0x800));
        }
        _ => panic!("not a set"),
    }
}

#[test]
fn everyone_is_the_everyone_role_and_a_member_its_pseudonym() {
    let cmd = overwrite(&["delete", "-c", "a", "-C", "c", "--target", "everyone"]);
    assert!(
        matches!(channel_request(&cmd), ChannelRequest::ChannelOverwriteDelete {
        target_type, target_id, ..
    } if target_type == "role" && target_id == "0".repeat(32))
    );
    let cmd = overwrite(&["delete", "-c", "a", "-C", "c", "--target", "member:beef"]);
    assert!(
        matches!(channel_request(&cmd), ChannelRequest::ChannelOverwriteDelete {
        target_type, target_id, ..
    } if target_type == "member" && target_id == "beef")
    );
}

#[test]
fn a_bad_target_or_mask_is_refused_before_sending() {
    for target in ["nobody", "role:xyz", "member:"] {
        let cmd = overwrite(&["delete", "-c", "a", "-C", "c", "--target", target]);
        assert!(request(&cmd).is_err(), "{target} refused");
    }
    let cmd = overwrite(&[
        "set", "-c", "a", "-C", "c", "--target", "everyone", "--deny", "lots",
    ]);
    assert!(request(&cmd).is_err());
    assert!(Cli::try_parse_from(["rekindle", "community", "channel", "overwrite", "set"]).is_err());
}
