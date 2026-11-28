//! A group call names the group DM it belongs to (plan E4.1): the start
//! is refused unless every invitee is in that group's roster, and a
//! receiver rings it as that group's call only when the sender and every
//! invitee are in its own copy of the roster.

use std::collections::HashMap;

use rekindle_codec::capnp_codec::direct::DirectBody;
use rekindle_crypto::identity::{key_fingerprint, short_key_fingerprint};
use rekindle_types::group_dm::GroupDmRosterMember;
use rekindle_types::subscription_events::CallParticipant;

use super::mocks::MockDeps;
use crate::signaling::deps::CallSignalingDeps;
use crate::signaling::GroupCallRegistry;
use crate::signaling::{handle_group_call_payload, start_group_call, CallSignalEvent};
use crate::state::CallKind;
use crate::CallError;

fn rosters(deps: &MockDeps, groups: &[(&str, &[&str])]) {
    *deps.group_rosters.lock() = groups
        .iter()
        .map(|(g, m)| {
            (
                (*g).to_owned(),
                m.iter()
                    .map(|&key| GroupDmRosterMember {
                        membership_generation: [1; 16],
                        identity: key.into(),
                        display_name: "Roster name".into(),
                        profile_record_key: String::new(),
                    })
                    .collect(),
            )
        })
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

fn incoming_names(deps: &MockDeps) -> (String, Vec<CallParticipant>) {
    deps.emitted_events()
        .into_iter()
        .find_map(|event| match event {
            CallSignalEvent::IncomingGroupCall {
                initiator_display_name,
                participants,
                ..
            } => Some((initiator_display_name, participants)),
            _ => None,
        })
        .expect("incoming group call")
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
    let participants = alice
        .emitted_events()
        .into_iter()
        .find_map(|event| match event {
            CallSignalEvent::GroupCallStarted { participants, .. } => Some(participants),
            _ => None,
        })
        .expect("outgoing group call");
    let unknown = participants.iter().find(|row| row.public_key == c).unwrap();
    assert_eq!(
        unknown.display_name,
        short_key_fingerprint(&hex::decode(&c).unwrap())
    );
    assert_ne!(unknown.display_name, c);
    assert!(!unknown.is_self);

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
    *alice.own_name.lock() = "Alice profile".into();
    *bob.own_name.lock() = "Bob profile".into();
    alice
        .friend_names
        .lock()
        .insert(a.clone(), "Wrong self alias".into());
    bob.friend_names.lock().insert(a.clone(), "Alice".into());
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
        vec![(
            call_id,
            Some("g2".to_owned()),
            vec![
                CallParticipant {
                    public_key: a.clone(),
                    display_name: "Alice profile".into(),
                    is_self: true,
                },
                CallParticipant {
                    public_key: b.clone(),
                    display_name: "Roster name".into(),
                    is_self: false,
                },
            ],
        )]
    );
    let (to, offer) = offers(&alice).pop().expect("one offer");
    assert_eq!(to, b);
    handle_group_call_payload(&*bob, &a, offer).await;
    assert_eq!(incoming_group(&bob), vec![Some("g2".to_owned())]);
    let (caller, participants) = incoming_names(&bob);
    assert_eq!(caller, "Alice");
    assert_eq!(
        participants,
        [
            CallParticipant {
                public_key: a,
                display_name: "Alice".into(),
                is_self: false,
            },
            CallParticipant {
                display_name: "Bob profile".into(),
                is_self: true,
                public_key: b,
            },
        ],
    );
}

#[tokio::test]
async fn incoming_group_names_use_the_receivers_roster_and_fingerprint() {
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
    let members: &[&str] = &[&a, &b, &c];
    rosters(&alice, &[("g", members)]);
    rosters(&bob, &[("g", members)]);
    {
        let mut rosters = bob.group_rosters.lock();
        let roster = rosters.get_mut("g").unwrap();
        roster[0].display_name = "Alice in Bob's roster".into();
        roster[2].display_name = c.get(..8).unwrap().into();
    }
    bob.friend_names.lock().extend([
        (a.clone(), key_fingerprint(&hex::decode(&a).unwrap())),
        (c.clone(), c.clone()),
    ]);
    start_group_call(
        &*alice,
        vec![b.clone(), c.clone()],
        CallKind::Video,
        Some("g".into()),
    )
    .await
    .expect("a member starts the group's call");
    let (_, offer) = offers(&alice)
        .into_iter()
        .find(|(to, _)| *to == b)
        .expect("Bob's offer");
    handle_group_call_payload(&*bob, &a, offer).await;
    let (caller, participants) = incoming_names(&bob);
    assert_eq!(caller, "Alice in Bob's roster");
    assert_eq!(participants[0].display_name, caller);
    assert_eq!(participants[2].public_key, c);
    assert_eq!(
        participants[2].display_name,
        short_key_fingerprint(&hex::decode(&c).unwrap())
    );
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

#[tokio::test]
async fn a_pending_invitee_is_refused_with_the_reason_its_view_serves() {
    use rekindle_types::group_dm::group_call_refused;

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
    // Carol holds only the invite to g1; its view serves this reason.
    carol.invited_groups.lock().insert("g1".to_owned());
    let served = group_call_refused(&carol.group_roster("g1").await.unwrap(), &c)
        .expect("a pending invitee may not call");
    assert_eq!(served, "accept the invite first");
    let refused = start_group_call(
        &*carol,
        vec![a.clone(), b.clone()],
        CallKind::Audio,
        Some("g1".into()),
    )
    .await
    .expect_err("a pending invitee's start is refused");
    assert_eq!(
        refused.to_string(),
        served,
        "the refusal is the served reason"
    );
    assert!(offers(&carol).is_empty(), "a refused start sends nothing");

    // Alice is on g1's roster with company: no reason, and the start goes.
    let roster = [a.as_str(), b.as_str(), c.as_str()];
    rosters(&alice, &[("g1", &roster)]);
    assert_eq!(
        group_call_refused(&alice.group_roster("g1").await.unwrap(), &a),
        None
    );
    start_group_call(
        &*alice,
        vec![b.clone(), c.clone()],
        CallKind::Audio,
        Some("g1".into()),
    )
    .await
    .expect("a full member starts the group's call");
    assert_eq!(offers(&alice).len(), 2);

    // Alone on the roster: the served reason, and the start refuses it too.
    rosters(&bob, &[("g1", &[b.as_str()])]);
    let alone = group_call_refused(&bob.group_roster("g1").await.unwrap(), &b).unwrap();
    let refused = start_group_call(&*bob, vec![a.clone()], CallKind::Audio, Some("g1".into()))
        .await
        .expect_err("no one else to call");
    assert_eq!(refused.to_string(), alone);
}
