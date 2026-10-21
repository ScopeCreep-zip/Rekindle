//! The group admission before a frame opens (plan E2.6): the friend gate
//! refused a group kind (0x0120-0x0124), alone or as a `PreKeyInit`'s first
//! payload, from a sender who is not blocked. The kind is in the header but
//! the group it names is sealed, so the sender is admitted when
//! `rekindle_dm::admit` admits it for one of the groups we hold; the
//! lifecycle step then checks the group the opened frame names. Friendship,
//! presence and profile kinds are never admitted here.

use rekindle_codec::capnp_codec::direct::DmTypeId;
use rekindle_dm::admission::{is_group_kind, GroupFrame};
use rekindle_dm::{AcceptedGroup, Admission};

use crate::operations::friend::FriendHost;

/// Whether `sender` (not blocked: the friend gate's block check ran
/// first) may deliver `kind`, alone or opening a session (`init`), as a
/// member of a group we hold. Fails closed when the groups are unreadable.
pub async fn admitted(host: &dyn FriendHost, sender: &str, kind: DmTypeId, init: bool) -> bool {
    if !is_group_kind(kind) {
        return false;
    }
    let groups = match host.group_dm() {
        Ok(deps) => match deps.groups().await {
            Ok(groups) => groups,
            Err(e) => {
                tracing::warn!(error = %e, "groups unreadable; failing closed");
                return false;
            }
        },
        Err(_) => return false,
    };
    let admitted = groups.iter().any(|g| {
        let held = [AcceptedGroup {
            group_id: &g.group_id,
            roster: &g.roster,
        }];
        let named = GroupFrame::Kind {
            kind,
            group_id: Some(&g.group_id),
        };
        let frame = if init {
            GroupFrame::PreKeyInit { first: &named }
        } else {
            named
        };
        rekindle_dm::admit(sender, false, &frame, &held) == Admission::Admit
    });
    if admitted {
        tracing::debug!(from = %sender, ?kind, "admitted as a group member");
    }
    admitted
}
