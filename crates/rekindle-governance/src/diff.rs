//! What changed between two merged governance states (plan E3.2 S9).
//!
//! Every peer merges the same signed, permission-checked op DAG into a
//! [`GovernanceState`], so a membership, role, channel or overwrite change
//! is read as the difference between the state a reader last projected and
//! the state just merged (Matrix reads `m.room.member` the same way: from
//! room state, never from an ephemeral notice). [`GovernanceState::diff`]
//! is that one comparison, pure and deterministic; the Host maps it to its
//! membership and governance events, and E3.5's ballot trigger reads its
//! departures and role changes.
//!
//! Effective permissions are not here: they are a function of a state at
//! an instant (`compute_permissions` reads timeouts against a clock), so a
//! reader evaluates them on both states at one `now`.

use std::collections::BTreeSet;

use rekindle_types::id::{ChannelId, PseudonymKey, RoleId};

use crate::state::GovernanceState;

/// One member's change of standing, in the order a reader shows them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemberChange {
    /// A joiner newly waits for approval (`AdmissionMode::ApprovalRequired`).
    JoinRequested {
        member: PseudonymKey,
        has_invite: bool,
    },
    /// A timeout was set, or a newer `TimeoutEntry` replaced a running one.
    TimedOut {
        member: PseudonymKey,
        duration_seconds: u64,
        reason: Option<String>,
        /// When the timeout ends (`started_at + duration_seconds`).
        until: u64,
    },
    /// A running timeout was lifted.
    TimeoutLifted { member: PseudonymKey },
    /// A member newly holds a seat.
    Joined { member: PseudonymKey },
    /// A ban was lifted.
    Unbanned { member: PseudonymKey },
    /// A member (or a never-joined pseudonym) was newly banned.
    Banned {
        member: PseudonymKey,
        by: Option<PseudonymKey>,
        reason: Option<String>,
    },
    /// A member was removed without a ban.
    Kicked {
        member: PseudonymKey,
        by: Option<PseudonymKey>,
        reason: Option<String>,
    },
    /// A member left of its own accord.
    Left { member: PseudonymKey },
}

impl MemberChange {
    /// The member the change is about.
    #[must_use]
    pub fn member(&self) -> &PseudonymKey {
        match self {
            Self::JoinRequested { member, .. }
            | Self::TimedOut { member, .. }
            | Self::TimeoutLifted { member }
            | Self::Joined { member }
            | Self::Unbanned { member }
            | Self::Banned { member, .. }
            | Self::Kicked { member, .. }
            | Self::Left { member } => member,
        }
    }
}

/// The difference between two governance states, each list in byte order
/// of its keys (membership changes grouped per member, members in order).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GovernanceDiff {
    /// Membership, timeout and ban changes.
    pub membership: Vec<MemberChange>,
    /// Members whose role set changed.
    pub member_roles: Vec<PseudonymKey>,
    /// Roles defined, deleted or edited.
    pub roles: Vec<RoleId>,
    /// Channels created, deleted or edited.
    pub channels: Vec<ChannelId>,
    /// Channels whose permission-overwrite set changed.
    pub channel_overwrites: Vec<ChannelId>,
}

impl GovernanceDiff {
    /// Whether nothing changed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    /// Whether the ban list changed.
    #[must_use]
    pub fn bans_changed(&self) -> bool {
        self.membership.iter().any(|c| {
            matches!(
                c,
                MemberChange::Banned { .. } | MemberChange::Unbanned { .. }
            )
        })
    }
}

impl GovernanceState {
    /// What changed from `self` (the older state) to `newer`.
    #[must_use]
    pub fn diff(&self, newer: &GovernanceState) -> GovernanceDiff {
        let roles = changed_keys(&self.roles, &newer.roles, |r| r.0);
        let channels = changed_keys(&self.channels, &newer.channels, |c| c.0);
        let member_roles = changed_keys(&self.role_assignments, &newer.role_assignments, |p| p.0);
        let overwritten: BTreeSet<[u8; 16]> = (self.overwrites.keys())
            .chain(newer.overwrites.keys())
            .map(|(c, _)| c.0)
            .collect();
        let channel_overwrites = overwritten
            .into_iter()
            .map(ChannelId)
            .filter(|c| overwrites_of(self, c) != overwrites_of(newer, c))
            .collect();
        GovernanceDiff {
            membership: membership_changes(self, newer),
            member_roles,
            roles,
            channels,
            channel_overwrites,
        }
    }
}

/// The keys present in either map whose value differs between them, in
/// byte order. A role set emptied of every role counts as unchanged from
/// an absent one.
fn changed_keys<K, V, B>(
    old: &std::collections::HashMap<K, V>,
    new: &std::collections::HashMap<K, V>,
    bytes: impl Fn(&K) -> B,
) -> Vec<K>
where
    K: Clone + Eq + std::hash::Hash,
    V: PartialEq + IsVacant,
    B: Ord,
{
    let mut keys: Vec<&K> = old.keys().chain(new.keys()).collect();
    keys.sort_by_key(|k| bytes(k));
    keys.dedup();
    keys.into_iter()
        .filter(|k| {
            let (a, b) = (old.get(*k), new.get(*k));
            a.filter(|v| !v.is_vacant()) != b.filter(|v| !v.is_vacant())
        })
        .cloned()
        .collect()
}

/// Whether a map value says no more than its absence.
trait IsVacant {
    fn is_vacant(&self) -> bool;
}

impl IsVacant for std::collections::HashSet<RoleId> {
    fn is_vacant(&self) -> bool {
        self.is_empty()
    }
}

impl IsVacant for crate::state::RoleState {
    fn is_vacant(&self) -> bool {
        false
    }
}

impl IsVacant for crate::state::ChannelState {
    fn is_vacant(&self) -> bool {
        false
    }
}

type OverwriteRow = (String, String, u64, u64);

/// `channel`'s overwrites in `state`, sorted.
fn overwrites_of(state: &GovernanceState, channel: &ChannelId) -> Vec<OverwriteRow> {
    let mut set: Vec<OverwriteRow> = state
        .overwrites
        .iter()
        .filter(|((c, _), _)| c == channel)
        .map(|((_, target), ow)| (target.clone(), ow.target_type.clone(), ow.allow, ow.deny))
        .collect();
    set.sort();
    set
}

/// The membership changes from `prev` to `next`, members in byte order.
fn membership_changes(prev: &GovernanceState, next: &GovernanceState) -> Vec<MemberChange> {
    let mut touched: Vec<&PseudonymKey> = (next.members.keys())
        .chain(prev.members.keys())
        .chain(prev.pending_members.keys())
        .chain(next.pending_members.keys())
        .chain(next.timeouts.keys())
        .chain(prev.timeouts.keys())
        .chain(next.bans.iter())
        .chain(prev.bans.iter())
        .collect();
    touched.sort_by_key(|p| p.0);
    touched.dedup();
    let mut out = Vec::new();
    for p in touched {
        member_changes(prev, next, p, &mut out);
    }
    out
}

/// `p`'s changes from `prev` to `next`, appended to `out`.
fn member_changes(
    prev: &GovernanceState,
    next: &GovernanceState,
    p: &PseudonymKey,
    out: &mut Vec<MemberChange>,
) {
    let member = || p.clone();
    let knew = prev.members.contains_key(p) || prev.pending_members.contains_key(p);
    let newly_banned = next.bans.contains(p) && !prev.bans.contains(p);
    let newly_departed = knew && !prev.is_departed(p) && next.is_departed(p);
    if let Some(pending) =
        (next.pending_members.get(p)).filter(|_| !prev.pending_members.contains_key(p))
    {
        out.push(MemberChange::JoinRequested {
            member: member(),
            has_invite: pending.has_invite,
        });
    }
    match (prev.timeouts.get(p), next.timeouts.get(p)) {
        (before, Some(now)) if before.map(|b| b.stamp) != Some(now.stamp) => {
            out.push(MemberChange::TimedOut {
                member: member(),
                duration_seconds: now.duration_seconds,
                reason: now.reason.clone(),
                until: now.started_at.saturating_add(now.duration_seconds),
            });
        }
        (Some(_), None) => out.push(MemberChange::TimeoutLifted { member: member() }),
        _ => {}
    }
    if next.members.contains_key(p) && !prev.members.contains_key(p) && !next.is_departed(p) {
        out.push(MemberChange::Joined { member: member() });
    }
    if prev.bans.contains(p) && !next.bans.contains(p) {
        out.push(MemberChange::Unbanned { member: member() });
    }
    if newly_banned || (newly_departed && next.removed.contains_key(p)) {
        let by = next.removed_by.get(p).cloned();
        let reason = next.removal_reasons.get(p).cloned();
        out.push(if next.bans.contains(p) {
            MemberChange::Banned {
                member: member(),
                by,
                reason,
            }
        } else {
            MemberChange::Kicked {
                member: member(),
                by,
                reason,
            }
        });
    } else if newly_departed {
        out.push(MemberChange::Left { member: member() });
    }
}

#[cfg(test)]
#[path = "diff_tests.rs"]
mod tests;
