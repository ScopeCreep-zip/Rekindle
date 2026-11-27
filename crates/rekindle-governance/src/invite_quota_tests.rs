#![allow(
    clippy::cast_possible_truncation,
    reason = "test fixture: the constants are compile-time known and well inside range; a fallible conversion here would add noise without adding safety"
)]

use std::collections::HashMap;

use super::*;
use crate::state::{CommunityPolicyState, InviteState};

fn pseudo(b: u8) -> PseudonymKey {
    PseudonymKey([b; 32])
}

fn invite(creator: PseudonymKey, lamport: u64) -> InviteState {
    InviteState {
        code_hash: format!("hash-{lamport}"),
        max_uses: 1,
        expires_at: None,
        secrets_record_key: String::new(),
        created: crate::fixture::stamp(lamport),
        creator_pseudonym: creator,
        uses: 0,
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
fn cap_uses_fixed_ceiling() {
    let mut state = GovernanceState::default();
    // One under the cap — may still mint.
    for i in 0..(MAX_ACTIVE_INVITES_PER_INVITER as u8 - 1) {
        let mut iid = [0u8; 16];
        iid[0] = i;
        state.invites.insert(iid, invite(pseudo(5), u64::from(i)));
    }
    assert!(check_active_invites_cap(&state, &pseudo(5)));

    // At the cap — the next mint is rejected.
    let mut iid_cap = [0u8; 16];
    iid_cap[0] = MAX_ACTIVE_INVITES_PER_INVITER as u8 - 1;
    state.invites.insert(
        iid_cap,
        invite(pseudo(5), u64::from(MAX_ACTIVE_INVITES_PER_INVITER)),
    );
    assert!(!check_active_invites_cap(&state, &pseudo(5)));
}

#[test]
fn cap_ignores_community_policy() {
    // A permissive raid-rate policy cannot raise the invite-count cap —
    // they govern different things.
    let mut state = GovernanceState {
        community_policy: Some(CommunityPolicyState {
            policy_text: None,
            max_joins_per_interval: 1000,
            join_interval_seconds: 600,
            stamp: crate::fixture::stamp(1),
        }),
        ..Default::default()
    };
    for i in 0..(MAX_ACTIVE_INVITES_PER_INVITER as u8) {
        let mut iid = [0u8; 16];
        iid[0] = i;
        state.invites.insert(iid, invite(pseudo(5), u64::from(i)));
    }
    assert!(!check_active_invites_cap(&state, &pseudo(5)));
}

#[test]
fn max_uses_cap_rejects_high_values() {
    assert!(!check_max_uses_cap(0), "no unlimited invites");
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
