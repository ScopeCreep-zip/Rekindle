//! The community member list's grouping (Discord's member list), decided
//! once by the Host so the desktop, TUI and CLI render the same groups.
//!
//! Hoisted role groups hold only members who are not offline, ordered by
//! role position (highest first); the other present members fall under
//! `Online`; every offline member, whatever their roles, falls under
//! `Offline`. Within a group the input order is kept.

use super::community::{CommunityMemberView, MemberGroup};
use crate::id::RoleId;
use crate::presence::SessionStatus;

/// A community role as the grouping needs it.
#[derive(Debug, Clone, Copy)]
pub struct GroupRole<'a> {
    pub id: RoleId,
    pub name: &'a str,
    pub position: u32,
    pub hoist: bool,
}

/// Whether a wire status counts as present: online, away or busy. Offline
/// and any status the wire does not define are not.
fn is_present(status: &str) -> bool {
    SessionStatus::from_wire_str(status).is_some_and(SessionStatus::is_visible_online)
}

/// Set every member's [`MemberGroup`] and order `members` group by group.
#[must_use]
pub fn group_members<'a>(
    mut members: Vec<CommunityMemberView>,
    roles: impl IntoIterator<Item = GroupRole<'a>>,
) -> Vec<CommunityMemberView> {
    // `@everyone` is never a group.
    let mut hoisted: Vec<(GroupRole<'a>, String)> = roles
        .into_iter()
        .filter(|r| r.hoist && r.id != RoleId::EVERYONE)
        .map(|r| (r, r.id.to_hex()))
        .collect();
    hoisted.sort_by(|(a, _), (b, _)| b.position.cmp(&a.position).then(a.id.0.cmp(&b.id.0)));
    let online_rank = hoisted.len();
    let mut ranked: Vec<(usize, CommunityMemberView)> = members
        .drain(..)
        .map(|mut m| {
            let (rank, group) = if !is_present(&m.status) {
                (online_rank + 1, MemberGroup::Offline)
            } else if let Some((i, (role, hex))) = hoisted
                .iter()
                .enumerate()
                .find(|(_, (_, hex))| m.role_ids.contains(hex))
            {
                (
                    i,
                    MemberGroup::Role {
                        id: hex.clone(),
                        name: role.name.to_owned(),
                    },
                )
            } else {
                (online_rank, MemberGroup::Online)
            };
            m.group = group;
            (rank, m)
        })
        .collect();
    ranked.sort_by_key(|(rank, _)| *rank);
    ranked.into_iter().map(|(_, m)| m).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::community::TargetModeration;

    fn member(name: &str, status: &str, role_ids: &[u8]) -> CommunityMemberView {
        CommunityMemberView {
            pseudonym_key: name.to_owned(),
            display_name: name.to_owned(),
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
            can_moderate_target: TargetModeration::default(),
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
}
