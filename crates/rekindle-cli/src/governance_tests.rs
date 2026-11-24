//! `community role …` and the moderation subcommands parse to the Host's
//! existing member requests, field for field; bad masks and targets are
//! refused before anything is sent.

use clap::Parser;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, MemberRequest};
use rekindle_types::governance::ExclusionGroupEdit;

use super::{moderate_request, role_created, role_request};
use crate::cli::{Cli, Command, CommunityCmd};

const GOV_ANCHOR: &str = "anchor-key";
const GOV_ROLE: &str = "0123456789abcdef0123456789abcdef";
const GOV_MEMBER: &str = "ca11ca11ca11ca11ca11ca11ca11ca11";

/// The Host request `rekindle community <args>` sends, or why it refuses.
fn request_for(args: &[&str]) -> anyhow::Result<IpcRequest> {
    let mut argv = vec!["rekindle", "community"];
    argv.extend_from_slice(args);
    let Some(Command::Community(cmd)) = Cli::try_parse_from(argv).unwrap().command else {
        panic!("expected a community command");
    };
    match cmd {
        CommunityCmd::Role(sub) => role_request(&sub),
        CommunityCmd::Moderate(sub) => moderate_request(&sub),
        _ => panic!("expected a role or moderation command"),
    }
}

fn assert_sends(args: &[&str], expected: MemberRequest) {
    let got = serde_json::to_value(request_for(args).unwrap()).unwrap();
    let want =
        serde_json::to_value(IpcRequest::Community(CommunityRequest::Members(expected))).unwrap();
    assert_eq!(got, want, "community {args:?}");
}

fn refusal(args: &[&str]) -> String {
    match request_for(args) {
        Ok(_) => panic!("community {args:?} was not refused"),
        Err(e) => e.to_string(),
    }
}

#[test]
fn role_create_sends_role_create_with_a_hex_mask() {
    assert_sends(
        &[
            "role",
            "create",
            "--community",
            GOV_ANCHOR,
            "--name",
            "Mods",
            "--permissions",
            "0x18",
            "--color",
            "#FF5733",
        ],
        MemberRequest::RoleCreate {
            community: GOV_ANCHOR.into(),
            name: "Mods".into(),
            permissions: 0x18,
            color: 0x00FF_5733,
            hoist: false,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
        },
    );
}

#[test]
fn role_create_takes_a_decimal_mask_and_no_color() {
    assert_sends(
        &[
            "role",
            "create",
            "--community",
            GOV_ANCHOR,
            "--name",
            "Mods",
            "--permissions",
            "8",
        ],
        MemberRequest::RoleCreate {
            community: GOV_ANCHOR.into(),
            name: "Mods".into(),
            permissions: 8,
            color: 0,
            hoist: false,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
        },
    );
}

#[test]
fn role_create_refuses_bad_masks_and_needs_one() {
    for bad in ["0xZZ", "1.5", "abc", "18446744073709551616", ""] {
        let base = [
            "role",
            "create",
            "--community",
            GOV_ANCHOR,
            "--name",
            "Mods",
        ];
        let msg = refusal(&[&base[..], &["--permissions", bad]].concat());
        assert!(msg.contains("permissions"), "{bad}: {msg}");
    }
    let missing = Cli::try_parse_from([
        "rekindle",
        "community",
        "role",
        "create",
        "--community",
        GOV_ANCHOR,
        "--name",
        "Mods",
    ]);
    assert!(missing.is_err(), "role create without --permissions parsed");
}

#[test]
fn role_edit_sends_only_what_changes() {
    assert_sends(
        &[
            "role",
            "edit",
            "--community",
            GOV_ANCHOR,
            "--role-id",
            GOV_ROLE,
            "--permissions",
            "0x4",
        ],
        MemberRequest::RoleUpdate {
            community: GOV_ANCHOR.into(),
            role_id: GOV_ROLE.into(),
            name: None,
            permissions: Some(4),
            color: None,
            position: None,
            hoist: None,
            mentionable: None,
            self_assignable: None,
            exclusion_group: ExclusionGroupEdit::Unchanged,
        },
    );
    let msg = refusal(&[
        "role",
        "edit",
        "--community",
        GOV_ANCHOR,
        "--role-id",
        GOV_ROLE,
    ]);
    assert!(msg.contains("nothing to change"), "{msg}");
}

#[test]
fn role_delete_list_assign_and_unassign_send_their_requests() {
    assert_sends(
        &[
            "role",
            "delete",
            "--community",
            GOV_ANCHOR,
            "--role-id",
            GOV_ROLE,
        ],
        MemberRequest::RoleDelete {
            community: GOV_ANCHOR.into(),
            role_id: GOV_ROLE.into(),
        },
    );
    assert_sends(
        &["role", "list", "--community", GOV_ANCHOR],
        MemberRequest::RoleList {
            community: GOV_ANCHOR.into(),
        },
    );
    let target = [
        "--community",
        GOV_ANCHOR,
        "--member",
        GOV_MEMBER,
        "--role-id",
        GOV_ROLE,
    ];
    assert_sends(
        &[&["role", "assign"][..], &target].concat(),
        MemberRequest::RoleAssign {
            community: GOV_ANCHOR.into(),
            member_pseudonym: GOV_MEMBER.into(),
            role_id: GOV_ROLE.into(),
        },
    );
    assert_sends(
        &[&["role", "unassign"][..], &target].concat(),
        MemberRequest::RoleUnassign {
            community: GOV_ANCHOR.into(),
            member_pseudonym: GOV_MEMBER.into(),
            role_id: GOV_ROLE.into(),
        },
    );
}

#[test]
fn role_targets_refuse_a_bad_role_id_or_an_empty_member() {
    let msg = refusal(&[
        "role",
        "delete",
        "--community",
        GOV_ANCHOR,
        "--role-id",
        "abc",
    ]);
    assert!(msg.contains("invalid role id"), "{msg}");
    let msg = refusal(&[
        "role",
        "assign",
        "--community",
        GOV_ANCHOR,
        "--member",
        GOV_MEMBER,
        "--role-id",
        "zz",
    ]);
    assert!(msg.contains("invalid role id"), "{msg}");
    let msg = refusal(&[
        "role",
        "unassign",
        "--community",
        GOV_ANCHOR,
        "--member",
        "  ",
        "--role-id",
        GOV_ROLE,
    ]);
    assert!(msg.contains("--member"), "{msg}");
}

#[test]
fn ban_unban_and_kick_send_their_requests() {
    assert_sends(
        &[
            "ban",
            "--community",
            GOV_ANCHOR,
            "--member",
            GOV_MEMBER,
            "--reason",
            "spam",
        ],
        MemberRequest::Ban {
            community: GOV_ANCHOR.into(),
            target_pseudonym: GOV_MEMBER.into(),
            reason: Some("spam".into()),
        },
    );
    assert_sends(
        &["unban", "--community", GOV_ANCHOR, "--member", GOV_MEMBER],
        MemberRequest::Unban {
            community: GOV_ANCHOR.into(),
            target_pseudonym: GOV_MEMBER.into(),
        },
    );
    assert_sends(
        &["kick", "--community", GOV_ANCHOR, "--member", GOV_MEMBER],
        MemberRequest::Kick {
            community: GOV_ANCHOR.into(),
            target_pseudonym: GOV_MEMBER.into(),
            reason: None,
        },
    );
    assert_sends(
        &[
            "timeout",
            "--community",
            GOV_ANCHOR,
            "--member",
            GOV_MEMBER,
            "--duration",
            "5m",
        ],
        MemberRequest::Timeout {
            community: GOV_ANCHOR.into(),
            target_pseudonym: GOV_MEMBER.into(),
            duration_seconds: 300,
            reason: None,
        },
    );
    assert_sends(
        &["bans", "--community", GOV_ANCHOR],
        MemberRequest::BanList {
            community: GOV_ANCHOR.into(),
        },
    );
}

#[test]
fn moderation_refuses_an_empty_member() {
    for cmd in ["ban", "unban", "kick"] {
        let msg = refusal(&[cmd, "--community", GOV_ANCHOR, "--member", ""]);
        assert!(msg.contains("--member"), "{cmd}: {msg}");
    }
}

#[test]
fn role_and_moderation_live_under_community_only() {
    for top in ["role", "moderate"] {
        assert!(
            Cli::try_parse_from(["rekindle", top, "list", "--community", GOV_ANCHOR]).is_err(),
            "`rekindle {top}` still parses"
        );
    }
}

#[test]
fn role_create_prints_the_new_role_id_as_role_id() {
    let printed = role_created(serde_json::json!(GOV_ROLE)).unwrap();
    assert_eq!(printed, serde_json::json!({ "roleId": GOV_ROLE }));
    assert!(role_created(serde_json::json!({ "ok": true })).is_err());
}
