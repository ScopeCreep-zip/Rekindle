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
#[path = "member_groups_tests.rs"]
mod tests;
