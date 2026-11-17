//! `member_transitions`: one event per membership change the merge makes,
//! carrying what the merge holds (A/B F7b-1, F7b-2).

use rekindle_governance::op::OpStamp;
use rekindle_governance::state::{GovernanceState, MemberState};
use rekindle_types::id::PseudonymKey;
use rekindle_types::subscription_events::MembershipEvent;

use super::member_transitions;

const C: &str = "community";

fn key(byte: u8) -> PseudonymKey {
    PseudonymKey([byte; 32])
}

fn member() -> MemberState {
    MemberState {
        member_record: "record".into(),
        member_record_owner: "owner".into(),
        joined: OpStamp::MIN,
    }
}

/// A state holding `members`, each with an op log.
fn with(members: &[u8]) -> GovernanceState {
    let mut state = GovernanceState::default();
    for &m in members {
        state.members.insert(key(m), member());
        state.op_log_records.insert(key(m), "log".into());
    }
    state
}

/// `prev` after `m`'s departure: its membership gone, its op log kept.
fn departed(prev: &GovernanceState, m: u8) -> GovernanceState {
    let mut next = prev.clone();
    next.members.remove(&key(m));
    next
}

/// `member_transitions` as JSON (the event types have no `PartialEq`).
fn transitions(
    prev: &GovernanceState,
    next: &GovernanceState,
    me: Option<&PseudonymKey>,
) -> serde_json::Value {
    json(member_transitions(C, prev, next, me, names))
}

fn json(events: Vec<MembershipEvent>) -> serde_json::Value {
    serde_json::to_value(events).expect("events serialize")
}

fn names(p: &PseudonymKey) -> Option<String> {
    (*p == key(1)).then(|| "Ann".to_owned())
}

#[test]
fn a_kick_is_one_kicked_naming_the_remover_and_the_reason() {
    let prev = with(&[1, 2, 3]);
    let mut next = departed(&prev, 2);
    next.removed.insert(key(2), OpStamp::MIN);
    next.removed_by.insert(key(2), key(1));
    next.removal_reasons.insert(key(2), "spam".into());
    let events = transitions(&prev, &next, Some(&key(3)));
    assert_eq!(
        events,
        json(vec![MembershipEvent::Kicked {
            community: C.into(),
            target_pseudonym: key(2).to_hex(),
            removed_by: Some(key(1).to_hex()),
            removed_by_name: Some("Ann".into()),
            reason: Some("spam".into()),
        }])
    );
    assert!(
        transitions(&next, &next, Some(&key(3))) == json(Vec::new()),
        "the same state projected again announces nothing"
    );
}

#[test]
fn a_ban_is_one_banned_and_our_own_removal_is_left_to_own_removal() {
    let prev = with(&[1, 2]);
    let mut next = departed(&prev, 2);
    next.bans.insert(key(2));
    next.removed.insert(key(2), OpStamp::MIN);
    next.removed_by.insert(key(2), key(1));
    let events = member_transitions(C, &prev, &next, Some(&key(1)), names);
    assert!(
        matches!(&events[..], [MembershipEvent::Banned { target_pseudonym, removed_by: Some(by), .. }]
            if *target_pseudonym == key(2).to_hex() && *by == key(1).to_hex()),
        "{events:?}"
    );
    assert!(
        transitions(&prev, &next, Some(&key(2))) == json(Vec::new()),
        "the banned member's own ban is the own-removal path's"
    );
    assert_eq!(
        transitions(&next, &prev, Some(&key(1))),
        json(vec![
            MembershipEvent::Joined {
                community: C.into(),
                pseudonym: key(2).to_hex(),
                display_name: String::new(),
                role_ids: Vec::new(),
            },
            MembershipEvent::Unbanned {
                community: C.into(),
                target_pseudonym: key(2).to_hex(),
            }
        ]),
        "a void ban restores the member: listed again, and unbanned"
    );
}

#[test]
fn a_leave_is_one_left_on_every_member_the_leaver_included() {
    let prev = with(&[1, 2]);
    let next = departed(&prev, 2);
    let left = json(vec![MembershipEvent::Left {
        community: C.into(),
        pseudonym: key(2).to_hex(),
    }]);
    assert_eq!(transitions(&prev, &next, Some(&key(1))), left);
    assert_eq!(
        transitions(&prev, &next, Some(&key(2))),
        left,
        "the leaver's own frontends see its leave"
    );
}

#[test]
fn a_join_is_one_joined_with_the_card_name() {
    let prev = with(&[2]);
    let next = with(&[1, 2]);
    assert_eq!(
        transitions(&prev, &next, Some(&key(2))),
        json(vec![MembershipEvent::Joined {
            community: C.into(),
            pseudonym: key(1).to_hex(),
            display_name: "Ann".into(),
            role_ids: Vec::new(),
        }])
    );
}

#[test]
fn a_departure_learnt_in_catch_up_of_a_member_never_shown_announces_nothing() {
    let prev = with(&[1]);
    let mut next = with(&[1]);
    next.op_log_records.insert(key(2), "log".into());
    assert!(transitions(&prev, &next, None) == json(Vec::new()));
}
