//! The creator set (plan E3.2 S7s.2, Q3, ADR 0011): every creator of the
//! cert holds creator rights from genesis; a further creator counts as a
//! member only after its own `MemberJoined` (lead decision (17)).

use std::collections::{BTreeSet, HashMap, HashSet};

use rekindle_types::governance::{AdmissionMode, GenesisCert};
use rekindle_types::permissions::{ADMINISTRATOR, ALL};

use super::*;
use crate::checkpoint::{eligible_set, eligible_set_hash};
use crate::fixture::{cert, merge_entries_under};
use crate::permissions::{compute_permissions, holds_creator_seat};
use crate::state::{GovernanceState, RoleState};
use crate::validate::target::{may_moderate, outranks, TargetAction};
use crate::validate::validate_write;

fn cert_with(primary: u8, further: &[u8]) -> GenesisCert {
    GenesisCert {
        additional_creators: further.iter().map(|b| pseudo(*b)).collect(),
        ..cert(&pseudo(primary), AdmissionMode::default())
    }
}

fn joined(b: u8) -> GovernanceEntry {
    GovernanceEntry::MemberJoined {
        member_record: format!("VLD0:record-{b}"),
        member_record_owner: format!("owner-{b}"),
        invite_proof: None,
        owner_proof: vec![0; 64],
        first_device: None,
    }
}

/// Creators 1 and 2, `pseudo(5)` an administrator at position 1,
/// `pseudo(3)` a plain member.
fn admin_state() -> GovernanceState {
    let mut roles = HashMap::new();
    roles.insert(
        role_id(1),
        RoleState {
            name: "admin".into(),
            permissions: ADMINISTRATOR,
            position: 1,
            color: 0,
            hoist: false,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
            stamp: crate::fixture::stamp(2),
        },
    );
    GovernanceState {
        creators: BTreeSet::from([pseudo(1), pseudo(2)]),
        roles,
        role_assignments: HashMap::from([(pseudo(5), HashSet::from([role_id(1)]))]),
        ..Default::default()
    }
}

#[test]
fn the_state_holds_the_certs_whole_creator_set() {
    let state = merge_entries_under(&cert_with(1, &[3, 2]), &[]);
    assert_eq!(
        state.creators,
        BTreeSet::from([pseudo(1), pseudo(2), pseudo(3)])
    );
}

#[test]
fn a_further_creator_can_do_what_the_creator_could() {
    let state = admin_state();
    let channel = GovernanceEntry::ChannelCreated {
        channel_id: channel_id(7),
        name: "ops".into(),
        channel_type: "text".into(),
        category_id: None,
        position: 0,
        parent_voice_channel_id: None,
    };
    assert_eq!(compute_permissions(&pseudo(2), None, &state, 0), ALL);
    assert!(validate_write(&pseudo(2), &channel, &state));
    assert!(
        outranks(&pseudo(2), &pseudo(5), &state),
        "a creator outranks an admin"
    );
    assert!(may_moderate(
        &pseudo(2),
        &pseudo(5),
        TargetAction::Kick,
        &state
    ));
    assert!(
        !outranks(&pseudo(5), &pseudo(2), &state),
        "no one outranks a creator"
    );
    assert!(!may_moderate(
        &pseudo(5),
        &pseudo(2),
        TargetAction::Ban,
        &state
    ));
}

#[test]
fn a_non_creator_still_cannot() {
    let state = admin_state();
    let channel = GovernanceEntry::ChannelCreated {
        channel_id: channel_id(7),
        name: "ops".into(),
        channel_type: "text".into(),
        category_id: None,
        position: 0,
        parent_voice_channel_id: None,
    };
    assert_ne!(compute_permissions(&pseudo(3), None, &state, 0), ALL);
    assert!(!validate_write(&pseudo(3), &channel, &state));
    assert!(!may_moderate(
        &pseudo(3),
        &pseudo(5),
        TargetAction::Kick,
        &state
    ));
    assert!(may_moderate(
        &pseudo(5),
        &pseudo(3),
        TargetAction::Kick,
        &state
    ));
}

#[test]
fn a_further_creator_is_a_member_only_after_its_member_joined() {
    let cert = cert_with(1, &[2]);
    let before = merge_entries_under(&cert, &[]);
    assert!(before.is_creator(&pseudo(2)));
    assert!(!before.is_counted_member(&pseudo(2)));
    assert_eq!(before.member_count(), 1);
    assert_eq!(
        before.counted_members().collect::<Vec<_>>(),
        vec![&pseudo(1)]
    );

    let after = merge_entries_under(
        &cert,
        &[
            (pseudo(2), vec![at(5, joined(2))]),
            (pseudo(3), vec![at(6, joined(3))]),
        ],
    );
    assert!(
        after.is_counted_member(&pseudo(2)),
        "a creator joins without an invite proof"
    );
    assert_eq!(after.member_count(), 2);
    assert!(
        !after.is_counted_member(&pseudo(3)) && !after.members.contains_key(&pseudo(3)),
        "a non-creator's join without an invite proof is refused"
    );
}

#[test]
fn the_first_checkpoint_hashes_the_whole_creator_set() {
    let set = eligible_set(&cert_with(1, &[3, 2]), None);
    assert_eq!(set, vec![pseudo(1), pseudo(2), pseudo(3)]);
    let alone = eligible_set(&cert_with(1, &[]), None);
    assert_ne!(eligible_set_hash(&set), eligible_set_hash(&alone));
}

#[test]
fn a_further_creator_joins_an_approval_community_without_approval() {
    let cert = GenesisCert {
        policy: AdmissionMode::ApprovalRequired,
        ..cert_with(1, &[2])
    };
    let channel = GovernanceEntry::ChannelCreated {
        channel_id: channel_id(7),
        name: "ops".into(),
        channel_type: "text".into(),
        category_id: None,
        position: 0,
        parent_voice_channel_id: None,
    };
    let state = merge_entries_under(
        &cert,
        &[(pseudo(2), vec![at(5, joined(2)), at(6, channel)])],
    );
    assert!(state.is_counted_member(&pseudo(2)));
    assert!(!state.pending_members.contains_key(&pseudo(2)));
    assert!(
        state.channels.contains_key(&channel_id(7)),
        "the approval gate passes a creator's op without an approval"
    );
}

#[test]
fn a_sole_creator_holds_the_seat_and_a_member_does_not() {
    let state = merge_entries_under(&cert_with(1, &[]), &[(pseudo(3), vec![at(5, joined(3))])]);
    assert!(holds_creator_seat(&pseudo(1), &state));
    assert!(!holds_creator_seat(&pseudo(3), &state));
}

#[test]
fn both_co_creators_hold_the_seat() {
    let state = merge_entries_under(&cert_with(1, &[2]), &[(pseudo(2), vec![at(5, joined(2))])]);
    assert!(holds_creator_seat(&pseudo(1), &state));
    assert!(holds_creator_seat(&pseudo(2), &state));
}

#[test]
fn a_removed_co_creator_no_longer_holds_the_seat() {
    let mut state =
        merge_entries_under(&cert_with(1, &[2]), &[(pseudo(2), vec![at(5, joined(2))])]);
    // What the merge's `MemberRemoved` arm leaves (`merge/membership.rs`).
    state.members.remove(&pseudo(2));
    state.removed.insert(pseudo(2), crate::fixture::stamp(9));
    assert!(state.is_creator(&pseudo(2)), "the cert's set is unchanged");
    assert!(!holds_creator_seat(&pseudo(2), &state));
    assert!(holds_creator_seat(&pseudo(1), &state));
}
