use super::*;
use crate::dto::community::MemberModeration;

fn member(name: &str, status: &str, role_ids: &[u8]) -> CommunityMemberView {
    CommunityMemberView {
        pseudonym_key: name.to_owned(),
        display_name: name.to_owned(),
        fingerprint: String::new(),
        custom_status: None,
        role_ids: role_ids.iter().map(|&b| RoleId([b; 16]).to_hex()).collect(),
        display_role: String::new(),
        status: status.to_owned(),
        timeout_until: None,
        bio: None,
        pronouns: None,
        theme_color: None,
        badges: Vec::new(),
        avatar_ref: None,
        banner_ref: None,
        location: None,
        last_active: 0,
        group: MemberGroup::Offline,
        moderation: MemberModeration::refused("You can't moderate yourself."),
        outranks: false,
    }
}

fn role(id: u8, name: &'static str, position: u32, hoist: bool) -> GroupRole<'static> {
    GroupRole {
        id: RoleId([id; 16]),
        name,
        position,
        hoist,
    }
}

fn roles() -> Vec<GroupRole<'static>> {
    vec![
        role(0, "@everyone", 0, true),
        role(1, "Mods", 1, true),
        role(2, "Admins", 5, true),
        role(3, "Quiet", 9, false),
    ]
}

fn names_and_groups(members: &[CommunityMemberView]) -> Vec<(&str, MemberGroup)> {
    members
        .iter()
        .map(|m| (m.display_name.as_str(), m.group.clone()))
        .collect()
}

fn role_group(id: u8, name: &str) -> MemberGroup {
    MemberGroup::Role {
        id: RoleId([id; 16]).to_hex(),
        name: name.to_owned(),
    }
}

#[test]
fn hoisted_roles_by_position_then_online_then_offline() {
    let grouped = group_members(
        vec![
            member("ModOnline", "online", &[0, 1]),
            member("UserAway", "away", &[0]),
            member("AdminBusy", "busy", &[0, 1, 2]),
            member("QuietOnline", "online", &[0, 3]),
            member("UserOffline", "offline", &[0]),
        ],
        roles(),
    );
    assert_eq!(
        names_and_groups(&grouped),
        vec![
            ("AdminBusy", role_group(2, "Admins")),
            ("ModOnline", role_group(1, "Mods")),
            ("UserAway", MemberGroup::Online),
            ("QuietOnline", MemberGroup::Online),
            ("UserOffline", MemberGroup::Offline),
        ]
    );
}

#[test]
fn hoisted_role_excludes_offline_members() {
    let grouped = group_members(
        vec![
            member("AdminOffline", "offline", &[0, 2]),
            member("AdminOnline", "online", &[0, 2]),
            member("AdminUnknown", "ingame", &[0, 2]),
        ],
        roles(),
    );
    assert_eq!(
        names_and_groups(&grouped),
        vec![
            ("AdminOnline", role_group(2, "Admins")),
            ("AdminOffline", MemberGroup::Offline),
            ("AdminUnknown", MemberGroup::Offline),
        ]
    );
}

#[test]
fn json_shape_of_each_group() {
    let json = |g: &MemberGroup| serde_json::to_value(g).unwrap();
    assert_eq!(
        json(&role_group(2, "Admins")),
        serde_json::json!({"kind": "role", "id": "02".repeat(16), "name": "Admins"})
    );
    assert_eq!(
        json(&MemberGroup::Online),
        serde_json::json!({"kind": "online"})
    );
    assert_eq!(
        json(&MemberGroup::Offline),
        serde_json::json!({"kind": "offline"})
    );
}
