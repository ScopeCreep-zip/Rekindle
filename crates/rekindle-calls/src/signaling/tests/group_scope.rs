//! A group call names the group DM it belongs to (plan E4.1): the start
//! is refused unless every invitee is in that group's roster, and a
//! receiver rings it as that group's call only when the sender and every
//! invitee are in its own copy of the roster.

use std::collections::HashMap;

use rekindle_codec::capnp_codec::direct::DirectBody;

use super::mocks::MockDeps;
use crate::signaling::deps::CallSignalingDeps;
use crate::signaling::GroupCallRegistry;
use crate::signaling::{handle_group_call_payload, start_group_call, CallSignalEvent};
use crate::state::CallKind;
use crate::CallError;

fn rosters(deps: &MockDeps, groups: &[(&str, &[&str])]) {
    *deps.group_rosters.lock() = groups
        .iter()
        .map(|(g, m)| ((*g).to_owned(), m.iter().map(|k| (*k).to_owned()).collect()))
        .collect::<HashMap<_, _>>();
}

fn offers(deps: &MockDeps) -> Vec<(String, DirectBody)> {
    deps.with(|s| s.sent.clone())
}

fn incoming_group(deps: &MockDeps) -> Vec<Option<String>> {
    deps.emitted_events()
        .into_iter()
        .filter_map(|e| match e {
            CallSignalEvent::IncomingGroupCall { group_id, .. } => Some(group_id),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_group_scoped_start_with_a_non_member_is_refused() {
    let (alice, bob, carol) = (
        MockDeps::with_identity([1; 32]),
        MockDeps::with_identity([2; 32]),
        MockDeps::with_identity([3; 32]),
    );
    let (a, b, c) = (
        alice.owner_key().unwrap(),
        bob.owner_key().unwrap(),
        carol.owner_key().unwrap(),
    );
    rosters(&alice, &[("g1", &[a.as_str(), b.as_str()])]);
    let refused = start_group_call(
        &*alice,
        vec![b.clone(), c.clone()],
        CallKind::Audio,
        Some("g1".into()),
    )
    .await;
    assert!(matches!(refused, Err(CallError::NotInGroup(ref why)) if why.contains(&c)));
    assert!(offers(&alice).is_empty(), "a refused start sends nothing");

    // A buddy-list group call (no group) invites anyone, as before.
    start_group_call(&*alice, vec![b.clone(), c.clone()], CallKind::Audio, None)
        .await
        .expect("a buddy-list group call starts");
    let sent = offers(&alice);
    assert_eq!(sent.len(), 2);
    assert!(sent.iter().all(|(_, body)| matches!(body,
        DirectBody::GroupCallOffer(o) if o.group_id.is_none())));
}

#[tokio::test]
async fn a_call_in_one_of_two_same_member_groups_rings_with_its_own_group_id() {
    let (alice, bob) = (
        MockDeps::with_identity([1; 32]),
        MockDeps::with_identity([2; 32]),
    );
    let (a, b) = (alice.owner_key().unwrap(), bob.owner_key().unwrap());
    let same: &[&str] = &[a.as_str(), b.as_str()];
    rosters(&alice, &[("g1", same), ("g2", same)]);
    rosters(&bob, &[("g1", same), ("g2", same)]);
    let call_id = start_group_call(&*alice, vec![b.clone()], CallKind::Audio, Some("g2".into()))
        .await
        .expect("a member starts the group's call");
    // The starter's own ring, with its group: what a group window's Cancel
    // ends (plan E4.1).
    let started: Vec<_> = alice
        .emitted_events()
        .into_iter()
        .filter_map(|e| match e {
            CallSignalEvent::GroupCallStarted {
                call_id,
                group_id,
                participants,
                ..
            } => Some((call_id, group_id, participants)),
            _ => None,
        })
        .collect();
    assert_eq!(
        started,
        vec![(call_id, Some("g2".to_owned()), vec![a.clone(), b.clone()])]
    );
    let (to, offer) = offers(&alice).pop().expect("one offer");
    assert_eq!(to, b);
    handle_group_call_payload(&*bob, &a, offer).await;
    assert_eq!(incoming_group(&bob), vec![Some("g2".to_owned())]);
}

#[tokio::test]
async fn a_forged_group_id_from_a_non_member_is_refused() {
    let (alice, bob, carol) = (
        MockDeps::with_identity([1; 32]),
        MockDeps::with_identity([2; 32]),
        MockDeps::with_identity([3; 32]),
    );
    let (a, b, c) = (
        alice.owner_key().unwrap(),
        bob.owner_key().unwrap(),
        carol.owner_key().unwrap(),
    );
    // Bob's group g1 is Alice and Bob; Carol claims it from her own copy.
    rosters(&bob, &[("g1", &[a.as_str(), b.as_str()])]);
    rosters(&carol, &[("g1", &[c.as_str(), b.as_str()])]);
    start_group_call(&*carol, vec![b.clone()], CallKind::Audio, Some("g1".into()))
        .await
        .expect("carol's own check passes on her forged roster");
    let (_, offer) = offers(&carol).pop().expect("one offer");
    handle_group_call_payload(&*bob, &c, offer).await;
    assert!(
        incoming_group(&bob).is_empty(),
        "a non-member's group call never rings"
    );
    assert!(bob.group_registry.list_ids().is_empty());
}
