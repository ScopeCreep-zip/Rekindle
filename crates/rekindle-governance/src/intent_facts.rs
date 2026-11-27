//! Governance facts an intent review states (plan U1.4; ADR 0019): who
//! manages the community and how many hold a role. Pure, like the rest of
//! the crate; the host passes the current member set from its registry.

use rekindle_types::id::{ChannelId, PseudonymKey, RoleId};
use rekindle_types::intent::{Escalation, Fact};
use rekindle_types::permissions::ADMINISTRATOR;

use crate::permissions::{channel_read_roster, compute_permissions};
use crate::state::GovernanceState;

/// Whether `member` can manage the community (the creator, or a role with
/// ADMINISTRATOR), and is not banned.
#[must_use]
pub fn can_manage(state: &GovernanceState, member: &PseudonymKey, now_unix: u64) -> bool {
    !state.bans.contains(member)
        && compute_permissions(member, None, state, now_unix) & ADMINISTRATOR != 0
}

/// Escalations for `me` leaving: E6 when no one else could manage the
/// community afterwards, or no one else is in it.
#[must_use]
pub fn leave_escalations(
    state: &GovernanceState,
    me: &PseudonymKey,
    members: &[PseudonymKey],
    now_unix: u64,
) -> Vec<Escalation> {
    let others: Vec<&PseudonymKey> = members
        .iter()
        .filter(|m| *m != me && !state.bans.contains(m))
        .collect();
    if others.is_empty() {
        return vec![Escalation::SoleMember];
    }
    if can_manage(state, me, now_unix) && !others.iter().any(|m| can_manage(state, m, now_unix)) {
        return vec![Escalation::LastAdmin];
    }
    Vec::new()
}

/// How many members hold `role`.
#[must_use]
pub fn role_holders(state: &GovernanceState, role: &RoleId) -> u32 {
    let held = state
        .role_assignments
        .values()
        .filter(|roles| roles.contains(role))
        .count();
    u32::try_from(held).unwrap_or(u32::MAX)
}

/// How many of `members` read `channel` (`channel_read_roster`): the
/// audience a channel deletion or a forward reaches.
#[must_use]
pub fn channel_members(
    state: &GovernanceState,
    channel: &ChannelId,
    members: &[PseudonymKey],
) -> u32 {
    u32::try_from(channel_read_roster(state, channel, members).len()).unwrap_or(u32::MAX)
}

/// The E7 escalation for deleting `channel`: its stored `messages`, and
/// the members who read it (`channel_members`). None when it holds neither.
#[must_use]
pub fn channel_delete_escalations(
    state: &GovernanceState,
    channel: &ChannelId,
    members: &[PseudonymKey],
    messages: u32,
) -> Vec<Escalation> {
    let members = channel_members(state, channel, members);
    if messages == 0 && members == 0 {
        return Vec::new();
    }
    vec![Escalation::ContainerNotEmpty { messages, members }]
}

/// A channel and the members who could read it: its community's merged
/// state and the registry's members.
#[derive(Debug, Clone, Copy)]
pub struct ChannelAudience<'a> {
    pub state: &'a GovernanceState,
    pub channel: &'a ChannelId,
    pub members: &'a [PseudonymKey],
}

/// The E4 escalation for forwarding a message from `from` into `to`: the
/// readers of each (`channel_members`), when the two read rosters differ.
/// A forward within one community to the same readers is none; pseudonyms
/// are scoped to their community, so a forward to another community always
/// reaches a different membership.
#[must_use]
pub fn forward_escalations(from: ChannelAudience<'_>, to: ChannelAudience<'_>) -> Vec<Escalation> {
    let mut from_roster = channel_read_roster(from.state, from.channel, from.members);
    let mut to_roster = channel_read_roster(to.state, to.channel, to.members);
    from_roster.sort_unstable_by_key(|k| k.0);
    from_roster.dedup();
    to_roster.sort_unstable_by_key(|k| k.0);
    to_roster.dedup();
    if from_roster == to_roster {
        return Vec::new();
    }
    vec![Escalation::CrossAudience {
        from_members: channel_members(from.state, from.channel, from.members),
        to_members: channel_members(to.state, to.channel, to.members),
    }]
}

/// What the review of publishing a successor states (plan E3.2 S7s.4,
/// ADR 0019's Destructive class): the publisher re-creates the community
/// as its sole creator, so `me` continues, every other creator of this
/// community leaves the creator set, and no history is copied.
#[must_use]
pub fn successor_publish_facts(state: &GovernanceState, me: &PseudonymKey) -> Vec<Fact> {
    let leaving = (state.creators.iter())
        .filter(|c| *c != me)
        .map(PseudonymKey::to_hex)
        .collect();
    vec![
        Fact::NewCreatorSet {
            creators: vec![me.to_hex()],
        },
        Fact::CreatorsLeave { creators: leaving },
        Fact::NoHistoryCopied,
    ]
}

#[cfg(test)]
#[path = "intent_facts_tests.rs"]
mod tests;
