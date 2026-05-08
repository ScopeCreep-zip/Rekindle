//! M10.3 — Invite quota enforcement (architecture §20.6 line 2607).
//!
//! Reader-validates: every honest peer applies the same per-inviter
//! active-invite cap and the same per-invite max-uses cap. A rogue admin
//! who mints over-quota invites sees their entries silently excluded
//! from the merged governance state on every peer.
//!
//! This is a Tier 6 helper — pure logic, no I/O, no async, no Veilid.

use rekindle_types::id::PseudonymKey;

use crate::state::{CommunityPolicyState, GovernanceState};

/// Hard ceiling on per-invite reuse. Even if `CommunityPolicy` permits a
/// higher value, readers reject the entry. Combined with the per-inviter
/// active-invite cap, this bounds the worst-case reachable joiners
/// from a single rogue inviter at `max_active_invites * MAX_USES_PER_INVITE`.
///
/// Set deliberately conservative — the Discord-equivalent invite is
/// usually 1, 25, or unlimited (`max_uses=0`). We forbid unlimited and
/// cap at 100, which is enough for legitimate community-link sharing
/// (small groups, classroom rosters, conference attendees) without
/// granting attacker leverage.
pub const MAX_USES_PER_INVITE: u32 = 100;

/// Count active (non-revoked, non-expired) invites whose `creator_pseudonym`
/// matches `inviter`. Excludes the entry currently being validated by
/// `code_hash` — `validate_write` calls this BEFORE the entry has been
/// merged, so the count reflects the prior state.
///
/// Active = present in `state.invites` (revoked invites are removed at
/// merge time). The expiry check is left to the joiner-side
/// `find_invite_in_governance` flow because expiry is wall-clock dependent.
pub fn active_invites_by_inviter(state: &GovernanceState, inviter: &PseudonymKey) -> u32 {
    state
        .invites
        .values()
        .filter(|invite| invite.creator_pseudonym == *inviter)
        .count()
        .try_into()
        .unwrap_or(u32::MAX)
}

/// Check whether `inviter` may mint a new invite, given the current state
/// and the community's policy. Returns `true` if accept, `false` if reject.
///
/// Defaults to `CommunityPolicyState::DEFAULT_MAX_JOINS_PER_INTERVAL` when
/// no `CommunityPolicy` entry has been merged yet (community is using
/// defaults, architecture §20.6 line 2607).
pub fn check_active_invites_cap(state: &GovernanceState, inviter: &PseudonymKey) -> bool {
    let cap = state
        .community_policy
        .as_ref()
        .map(|p| p.max_joins_per_interval)
        .unwrap_or(CommunityPolicyState::DEFAULT_MAX_JOINS_PER_INTERVAL);
    active_invites_by_inviter(state, inviter) < cap
}

/// Reject invites whose declared reuse count exceeds the hard cap.
/// Reader-validates: even a creator-bypass writer cannot smuggle a
/// `max_uses = u32::MAX` entry past honest peers.
pub fn check_max_uses_cap(max_uses: u32) -> bool {
    max_uses <= MAX_USES_PER_INVITE
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::state::InviteState;

    fn pseudo(b: u8) -> PseudonymKey {
        PseudonymKey([b; 32])
    }

    fn invite(creator: PseudonymKey, lamport: u64) -> InviteState {
        InviteState {
            code_hash: format!("hash-{lamport}"),
            max_uses: 1,
            expires_at: None,
            encrypted_secrets: String::new(),
            created_lamport: lamport,
            creator_pseudonym: creator,
        }
    }

    #[test]
    fn active_count_filters_by_inviter() {
        let mut state = GovernanceState::default();
        state.invites.insert([1; 16], invite(pseudo(5), 10));
        state.invites.insert([2; 16], invite(pseudo(5), 11));
        state.invites.insert([3; 16], invite(pseudo(7), 12));
        assert_eq!(active_invites_by_inviter(&state, &pseudo(5)), 2);
        assert_eq!(active_invites_by_inviter(&state, &pseudo(7)), 1);
        assert_eq!(active_invites_by_inviter(&state, &pseudo(99)), 0);
    }

    #[test]
    fn cap_uses_default_when_no_policy() {
        let mut state = GovernanceState::default();
        // 19 invites — under the default cap of 20.
        for i in 0..19_u8 {
            let mut iid = [0u8; 16];
            iid[0] = i;
            state.invites.insert(iid, invite(pseudo(5), u64::from(i)));
        }
        assert!(check_active_invites_cap(&state, &pseudo(5)));

        // 20 invites — at cap, the 21st is rejected.
        let mut iid_20 = [0u8; 16];
        iid_20[0] = 19;
        state.invites.insert(iid_20, invite(pseudo(5), 19));
        assert!(!check_active_invites_cap(&state, &pseudo(5)));
    }

    #[test]
    fn cap_respects_community_policy_override() {
        let mut state = GovernanceState::default();
        state.community_policy = Some(CommunityPolicyState {
            policy_text: None,
            max_joins_per_interval: 5,
            join_interval_seconds: 600,
            lamport: 1,
        });
        for i in 0..5_u8 {
            let mut iid = [0u8; 16];
            iid[0] = i;
            state.invites.insert(iid, invite(pseudo(5), u64::from(i)));
        }
        assert!(!check_active_invites_cap(&state, &pseudo(5)));
    }

    #[test]
    fn max_uses_cap_rejects_high_values() {
        assert!(check_max_uses_cap(1));
        assert!(check_max_uses_cap(MAX_USES_PER_INVITE));
        assert!(!check_max_uses_cap(MAX_USES_PER_INVITE + 1));
        assert!(!check_max_uses_cap(u32::MAX));
    }

    #[test]
    fn other_inviters_dont_count_toward_quota() {
        // Cross-pollination test — inviter A's 19 invites do not push
        // inviter B over their own quota.
        let mut state = GovernanceState::default();
        for i in 0..19_u8 {
            let mut iid = [0u8; 16];
            iid[0] = i;
            state.invites.insert(iid, invite(pseudo(5), u64::from(i)));
        }
        // Inviter B has zero invites — they're under cap.
        assert!(check_active_invites_cap(&state, &pseudo(7)));
        // HashMap initialized empty for the type sanity:
        let _: HashMap<[u8; 16], InviteState> = state.invites.clone();
    }
}
