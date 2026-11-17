//! Membership verdicts from the merged governance (plan E3.2 S9, A/B
//! F7b-1/F7b-2), on both modes: who joined, left, was kicked, banned or
//! unbanned is what the merge of the signed, permission-checked op DAG
//! says, read as the difference between the state a frontend was last
//! shown and the state just merged. Each transition is published once
//! (a later projection of the same state differs in nothing), with the
//! full payload the merge holds: the remover and the reason, the leaver's
//! own departure to its own frontends.
//!
//! The gossip notices (`Kick`, `Ban`, `MemberLeave`, `MemberJoined`, ...)
//! publish no membership event (`rekindle-transport`
//! `control_into_event`): a gossip frame is a hint, not authenticated for
//! policy, and it carries neither the remover nor the reason, so a frontend
//! taking the first event it saw got a removal with no reason (F7b-2) and a
//! member whose gossip missed a leave never saw it (F7b-1). Matrix makes
//! the same split: membership is a state event in the room DAG
//! (`m.room.member`), never an ephemeral EDU.
//!
//! Our own kick or ban is not published here: the own-removal path
//! (`own_removal.rs`) announces it once, with the remover's name kept
//! across the teardown.

use rekindle_governance::state::GovernanceState;
use rekindle_types::id::PseudonymKey;
use rekindle_types::subscription_events::{MembershipEvent, SubscriptionEvent};

use super::HostGovernance;

impl HostGovernance {
    /// What changed in membership between `prev` (the state last
    /// projected, `None` on the first projection, which is a baseline and
    /// announces nothing) and `next`.
    pub(super) fn member_transitions_of(
        &self,
        community_id: &str,
        prev: Option<&GovernanceState>,
        next: &GovernanceState,
        me: Option<&PseudonymKey>,
    ) -> Vec<MembershipEvent> {
        let Some(prev) = prev else {
            return Vec::new();
        };
        let communities = &self.ctx.communities;
        member_transitions(community_id, prev, next, me, |p| {
            communities.removal_name_of(community_id, p)
        })
    }

    /// Publish `events` (from [`Self::member_transitions_of`]).
    pub(super) fn publish_member_transitions(&self, events: Vec<MembershipEvent>) {
        for event in events {
            self.ctx.publish_event(SubscriptionEvent::Membership(event));
        }
    }
}

/// The membership events that take `prev` to `next`, in pseudonym order.
/// `name_of` gives a member's card or profile name (a joiner's, a
/// remover's). Our own kick or ban (`me`) is left to the own-removal path.
pub(super) fn member_transitions(
    community_id: &str,
    prev: &GovernanceState,
    next: &GovernanceState,
    me: Option<&PseudonymKey>,
    name_of: impl Fn(&PseudonymKey) -> Option<String>,
) -> Vec<MembershipEvent> {
    let community = || community_id.to_owned();
    let mut touched: Vec<&PseudonymKey> = next
        .members
        .keys()
        .chain(prev.members.keys())
        .chain(prev.pending_members.keys())
        .chain(next.bans.iter())
        .chain(prev.bans.iter())
        .collect();
    touched.sort_by_key(|p| p.0);
    touched.dedup();

    let mut out = Vec::new();
    for p in touched {
        let knew = prev.members.contains_key(p) || prev.pending_members.contains_key(p);
        let newly_banned = next.bans.contains(p) && !prev.bans.contains(p);
        let newly_departed = knew && !prev.is_departed(p) && next.is_departed(p);
        let mine = me == Some(p);
        if next.members.contains_key(p) && !prev.members.contains_key(p) && !next.is_departed(p) {
            out.push(MembershipEvent::Joined {
                community: community(),
                pseudonym: p.to_hex(),
                display_name: name_of(p).unwrap_or_default(),
                // The legacy numeric role list: a member's roles are 16-byte
                // ids, published by `RolesChanged` once assigned.
                role_ids: Vec::new(),
            });
        }
        if prev.bans.contains(p) && !next.bans.contains(p) {
            out.push(MembershipEvent::Unbanned {
                community: community(),
                target_pseudonym: p.to_hex(),
            });
        }
        if newly_banned || (newly_departed && next.removed.contains_key(p)) {
            if mine {
                continue;
            }
            let removed_by = next.removed_by.get(p);
            let (target_pseudonym, reason) = (p.to_hex(), next.removal_reasons.get(p).cloned());
            let removed_by_name = removed_by.and_then(&name_of);
            let removed_by = removed_by.map(PseudonymKey::to_hex);
            out.push(if next.bans.contains(p) {
                MembershipEvent::Banned {
                    community: community(),
                    target_pseudonym,
                    removed_by,
                    removed_by_name,
                    reason,
                }
            } else {
                MembershipEvent::Kicked {
                    community: community(),
                    target_pseudonym,
                    removed_by,
                    removed_by_name,
                    reason,
                }
            });
        } else if newly_departed {
            out.push(MembershipEvent::Left {
                community: community(),
                pseudonym: p.to_hex(),
            });
        }
    }
    out
}

#[cfg(test)]
#[path = "member_events_tests.rs"]
mod tests;
