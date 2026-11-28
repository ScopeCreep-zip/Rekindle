//! `member_transitions`: one event per membership change the merge makes,
//! carrying what the merge holds (A/B F7b-1, F7b-2).

use rekindle_governance::op::OpStamp;
use rekindle_governance::state::{GovernanceState, MemberState};
use rekindle_types::id::PseudonymKey;
use rekindle_types::subscription_events::MembershipEvent;

use super::{departures_to_rotate, member_transitions};

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
    json(member_transitions(C, &prev.diff(next), me, names, names))
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
    let events = member_transitions(C, &prev.diff(&next), Some(&key(1)), names, names);
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
                display_name: None,
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

/// ADR 0018: a merged leave of another member starts exactly one departure
/// rotation (the plain, non-holding kind); our own leave, a kick or a ban
/// (the moderation path's rotation) and a join start none.
#[test]
fn a_merged_leave_of_another_member_is_the_one_departure_rotated() {
    let prev = with(&[1, 2, 3]);
    let mut next = departed(&prev, 2);
    next.members.remove(&key(3));
    next.removed.insert(key(3), OpStamp::MIN);
    next.removed_by.insert(key(3), key(1));
    let events = member_transitions(C, &prev.diff(&next), Some(&key(1)), names, names);
    let leaver = key(2).to_hex();
    assert_eq!(
        departures_to_rotate(&events, Some(&key(1))),
        vec![leaver.as_str()]
    );
    let own = member_transitions(
        C,
        &prev.diff(&departed(&prev, 1)),
        Some(&key(1)),
        names,
        names,
    );
    assert!(
        departures_to_rotate(&own, Some(&key(1))).is_empty(),
        "our own leave rotates nothing of ours"
    );
    let join = member_transitions(
        C,
        &with(&[2]).diff(&with(&[1, 2])),
        Some(&key(2)),
        names,
        names,
    );
    assert!(departures_to_rotate(&join, Some(&key(2))).is_empty());
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
            display_name: Some("Ann".into()),
            role_ids: Vec::new(),
        }])
    );
}

/// Plan E3.2 F9-2: a joiner's name is its verified card's alone. Before the
/// card is read the Joined carries none (the name follows as
/// `MemberCardChanged`), even when a stored profile row holds one: those
/// rows are not all signed by the member.
#[test]
fn a_join_before_the_card_is_read_carries_no_name_not_the_profile_row() {
    let prev = with(&[2]);
    let next = with(&[1, 2]);
    let no_card = |_: &PseudonymKey| None;
    let events = member_transitions(C, &prev.diff(&next), Some(&key(2)), no_card, names);
    assert_eq!(
        json(events),
        json(vec![MembershipEvent::Joined {
            community: C.into(),
            pseudonym: key(1).to_hex(),
            display_name: None,
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

/// Plan E3.2 S9: a joiner first held pending under `ApprovalRequired` is
/// one `JoinRequested`, named by its card and saying whether it carried an
/// invite; still pending on the next merge, it is nothing.
#[test]
fn a_pending_joiner_is_one_join_requested() {
    let prev = with(&[2]);
    let mut next = prev.clone();
    next.pending_members.insert(
        key(1),
        rekindle_governance::state::PendingMemberState {
            member_record: "record".into(),
            member_record_owner: "owner".into(),
            has_invite: true,
            stamp: OpStamp::MIN,
        },
    );
    assert_eq!(
        transitions(&prev, &next, None),
        serde_json::json!([{ "joinRequested": {
            "community": C, "pseudonym": key(1).to_hex(), "displayName": "Ann", "hasInvite": true,
        }}])
    );
    assert_eq!(transitions(&next, &next, None), serde_json::json!([]));
}

/// Plan E3.2 S9: a timeout is `TimedOut` with its reason and the end; its
/// lifting is `TimeoutRemoved` and a lifted status, ours included.
#[test]
fn a_timeout_and_its_lifting_are_published_with_the_reason() {
    let prev = with(&[1, 2]);
    let mut next = prev.clone();
    next.timeouts.insert(
        key(2),
        rekindle_governance::state::TimeoutState {
            duration_seconds: 60,
            started_at: 100,
            reason: Some("spam".into()),
            stamp: OpStamp::MIN,
        },
    );
    let p = key(2).to_hex();
    assert_eq!(
        transitions(&prev, &next, Some(&key(2))),
        serde_json::json!([
            { "timedOut": { "community": C, "targetPseudonym": p, "durationSeconds": 60, "reason": "spam" } },
            { "timeoutStatusChanged": { "community": C, "pseudonym": p, "timeoutUntil": 160 } },
        ])
    );
    assert_eq!(
        transitions(&next, &prev, Some(&key(2))),
        serde_json::json!([
            { "timeoutRemoved": { "community": C, "targetPseudonym": p } },
            { "timeoutStatusChanged": { "community": C, "pseudonym": p, "timeoutUntil": null } },
        ])
    );
}
