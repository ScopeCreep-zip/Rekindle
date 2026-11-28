//! Admission of group members who are not friends (plan E2.6). The friend
//! gate admits accepted friends; this one admits a sender on the signed
//! roster of a group we accepted, and only for:
//!
//! - the group kinds 0x0120-0x0124, naming that group;
//! - 0x0107 `StreamRecord` naming that group (the record it announces is
//!   stored as the sender's own chain there, never another member's);
//! - 0x0160 `DisappearingTimer` naming that group (plan E7.1);
//! - the `PreKeyInit` that opens the pair's session, when its first
//!   payload is itself admitted;
//! - a lobby kind 0x0127-0x012B (plan E7.2 G6) opened from an entry of
//!   that group's chain ([`GroupFrame::Lobby`]); never as a frame of its
//!   own, nor as a `PreKeyInit`'s first payload.
//!
//! Never friendship, presence or profile frames, and never a blocked
//! sender. Leaving deletes the group (so it is no longer accepted) and a
//! member who left is off our roster: either ends the admission.

use rekindle_codec::capnp_codec::direct::{DmTypeId, GroupId};

use crate::group::Roster;

/// A group we accepted (or created), as admission reads it.
#[derive(Debug, Clone, Copy)]
pub struct AcceptedGroup<'a> {
    pub group_id: &'a GroupId,
    pub roster: &'a Roster,
}

/// What admission reads of an inbound frame.
#[derive(Debug, Clone, Copy)]
pub enum GroupFrame<'a> {
    /// A frame of `kind` naming `group_id` (the group kinds carry one).
    Kind {
        kind: DmTypeId,
        group_id: Option<&'a GroupId>,
    },
    /// A `StreamRecord` announcing `record_key`, for `group_id` when it is
    /// a group chain.
    StreamRecord { group_id: Option<&'a GroupId> },
    /// A `DisappearingTimer` (plan E7.1), for `group_id` when it is a
    /// group's (a DM's needs the friend gate).
    DisappearingTimer { group_id: Option<&'a GroupId> },
    /// A `PreKeyInit` whose first payload is `first`.
    PreKeyInit { first: &'a GroupFrame<'a> },
    /// A lobby kind (plan E7.2 G6) opened from the sender's entry on its
    /// chain in `group_id` (the `GroupContent` entry's group). The sender
    /// must be on that group's roster now: a member who left, or a writer
    /// never on it, is dropped, so no row is ever folded for them.
    Lobby {
        kind: DmTypeId,
        group_id: &'a GroupId,
    },
}

/// Admission's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    Admit,
    /// The sender is blocked.
    Blocked,
    /// Not a kind this gate admits (friendship, presence, profile, ...).
    KindRefused,
    /// The sender is on no accepted group's roster as the frame needs.
    NotOnRoster,
}

/// Whether `kind` is one of the group kinds 0x0120-0x0124.
#[must_use]
pub fn is_group_kind(kind: DmTypeId) -> bool {
    matches!(
        kind,
        DmTypeId::GroupDmInvite
            | DmTypeId::GroupDmAccept
            | DmTypeId::GroupDmDecline
            | DmTypeId::GroupDmLeave
            | DmTypeId::GroupDmRosterGrow
            | DmTypeId::GroupContent
    )
}

/// Whether `kind` is one of the lobby kinds 0x0127-0x012B (plan E7.2 G6),
/// which travel only inside a `GroupContent` entry's inner.
#[must_use]
pub fn is_lobby_kind(kind: DmTypeId) -> bool {
    matches!(
        kind,
        DmTypeId::LobbyOpen
            | DmTypeId::LobbyJoin
            | DmTypeId::LobbyLeave
            | DmTypeId::LobbyLaunch
            | DmTypeId::LobbyClose
    )
}

fn group_of<'a>(groups: &'a [AcceptedGroup<'a>], id: &GroupId) -> Option<&'a AcceptedGroup<'a>> {
    groups.iter().find(|g| g.group_id == id)
}

/// Whether `sender` (not a friend, or a friend whose gate refused this
/// frame) is admitted for `frame` by the groups we accepted.
#[must_use]
pub fn admit(
    sender: &str,
    blocked: bool,
    frame: &GroupFrame<'_>,
    groups: &[AcceptedGroup<'_>],
) -> Admission {
    if blocked {
        return Admission::Blocked;
    }
    match frame {
        GroupFrame::Kind { kind, group_id } => {
            if !is_group_kind(*kind) {
                return Admission::KindRefused;
            }
            let on_roster = group_id
                .and_then(|id| group_of(groups, id))
                .is_some_and(|g| g.roster.contains(sender));
            if on_roster {
                Admission::Admit
            } else {
                Admission::NotOnRoster
            }
        }
        GroupFrame::StreamRecord { group_id } | GroupFrame::DisappearingTimer { group_id } => {
            let Some(id) = group_id else {
                return Admission::KindRefused;
            };
            if group_of(groups, id).is_some_and(|g| g.roster.contains(sender)) {
                Admission::Admit
            } else {
                Admission::NotOnRoster
            }
        }
        GroupFrame::Lobby { kind, group_id } => {
            if !is_lobby_kind(*kind) {
                return Admission::KindRefused;
            }
            if group_of(groups, group_id).is_some_and(|g| g.roster.contains(sender)) {
                Admission::Admit
            } else {
                Admission::NotOnRoster
            }
        }
        GroupFrame::PreKeyInit { first } => match first {
            GroupFrame::PreKeyInit { .. } | GroupFrame::Lobby { .. } => Admission::KindRefused,
            inner => admit(sender, blocked, inner, groups),
        },
    }
}

#[cfg(test)]
#[path = "admission/tests.rs"]
mod tests;
