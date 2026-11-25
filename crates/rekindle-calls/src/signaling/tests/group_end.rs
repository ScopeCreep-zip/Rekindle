//! Ending a group call reaches the people it rang (finding GV3): a caller's
//! cancel while the call still rings clears every invitee's ring, the
//! caller's own call ends, and a joined member's leave reaches the others.

use rekindle_codec::capnp_codec::direct::{DirectBody, GroupCallParticipantLeft};

use super::mocks::MockDeps;
use crate::signaling::{
    accept_group_call, end_group_call, handle_group_call_payload, start_group_call,
    CallSignalEvent, GroupCallRegistry,
};
use crate::state::CallKind;

fn key(secret: [u8; 32]) -> String {
    rekindle_crypto::Identity::from_secret_bytes(&secret).public_key_hex()
}

/// What `from` sent since the last take, emptied.
fn take_sent(from: &MockDeps) -> Vec<(String, DirectBody)> {
    from.with(|s| std::mem::take(&mut s.sent))
}

/// Deliver everything `from` (whose key is `from_key`) sent to `to` (key `to_key`).
async fn deliver(sent: &[(String, DirectBody)], from_key: &str, to: &MockDeps, to_key: &str) {
    for (peer, body) in sent {
        if peer == to_key {
            handle_group_call_payload(to, from_key, body.clone()).await;
        }
    }
}

fn ended(deps: &MockDeps, call: &str) -> bool {
    deps.emitted_events()
        .iter()
        .any(|e| matches!(e, CallSignalEvent::GroupCallEnded { call_id, .. } if call_id == call))
}

fn left(deps: &MockDeps, call: &str, who: &str) -> bool {
    deps.emitted_events().iter().any(|e| {
        matches!(e, CallSignalEvent::GroupCallParticipantLeft { call_id, peer_public_key, .. }
            if call_id == call && peer_public_key == who)
    })
}

#[tokio::test]
async fn cancelling_a_ringing_group_call_clears_every_invitees_ring() {
    let (bob, alice, carol) = (
        MockDeps::with_identity([1; 32]),
        MockDeps::with_identity([2; 32]),
        MockDeps::with_identity([3; 32]),
    );
    let (bk, ak, ck) = (key([1; 32]), key([2; 32]), key([3; 32]));
    let call = start_group_call(
        bob.as_ref(),
        vec![ak.clone(), ck.clone()],
        CallKind::Audio,
        None,
    )
    .await
    .expect("start");
    let offers = take_sent(&bob);
    deliver(&offers, &bk, &alice, &ak).await;
    deliver(&offers, &bk, &carol, &ck).await;
    assert!(alice.group_registry.contains(&call), "alice rings");
    assert!(carol.group_registry.contains(&call), "carol rings");

    end_group_call(bob.as_ref(), &call, Some("cancelled".into()))
        .await
        .expect("cancel");
    assert!(
        !bob.group_registry.contains(&call),
        "the caller's call is gone"
    );
    assert!(
        ended(&bob, &call),
        "the caller's window learns its call ended"
    );

    let cancels = take_sent(&bob);
    for (invitee, invitee_key) in [(&alice, &ak), (&carol, &ck)] {
        assert!(
            cancels.iter().any(|(p, b)| p == invitee_key
                && matches!(b, DirectBody::GroupCallParticipantLeft(l) if l.participant_pubkey == bk)),
            "the cancel goes to {invitee_key}: {cancels:?}"
        );
        deliver(&cancels, &bk, invitee, invitee_key).await;
        assert!(
            !invitee.group_registry.contains(&call),
            "the ring is dropped"
        );
        assert!(
            ended(invitee, &call),
            "the invitee's window goes back to Call"
        );
    }
}

#[tokio::test]
async fn a_joined_members_leave_reaches_the_other_participant() {
    let (bob, alice) = (
        MockDeps::with_identity([1; 32]),
        MockDeps::with_identity([2; 32]),
    );
    let (bk, ak) = (key([1; 32]), key([2; 32]));
    let call = start_group_call(bob.as_ref(), vec![ak.clone()], CallKind::Audio, None)
        .await
        .expect("start");
    deliver(&take_sent(&bob), &bk, &alice, &ak).await;
    accept_group_call(alice.as_ref(), &call)
        .await
        .expect("join");
    deliver(&take_sent(&alice), &ak, &bob, &bk).await;

    end_group_call(alice.as_ref(), &call, Some("left".into()))
        .await
        .expect("leave");
    deliver(&take_sent(&alice), &ak, &bob, &bk).await;
    assert!(left(&bob, &call, &ak), "bob learns alice left");
    assert!(bob.group_registry.contains(&call), "bob's call goes on");

    end_group_call(bob.as_ref(), &call, None)
        .await
        .expect("end");
    assert!(
        take_sent(&bob).iter().any(|(p, _)| p == &ak),
        "the initiator's end reaches the member who joined"
    );
}

#[tokio::test]
async fn only_the_leaver_can_say_it_left() {
    let (bob, alice) = (
        MockDeps::with_identity([1; 32]),
        MockDeps::with_identity([2; 32]),
    );
    let (bk, ak) = (key([1; 32]), key([2; 32]));
    let call = start_group_call(bob.as_ref(), vec![ak.clone()], CallKind::Audio, None)
        .await
        .expect("start");
    deliver(&take_sent(&bob), &bk, &alice, &ak).await;
    let forged = DirectBody::GroupCallParticipantLeft(GroupCallParticipantLeft {
        call_id: call.clone(),
        participant_pubkey: bk.clone(),
        reason: String::new(),
    });
    handle_group_call_payload(alice.as_ref(), &"ee".repeat(32), forged).await;
    assert!(
        alice.group_registry.contains(&call),
        "a forged cancel keeps the ring"
    );
    assert!(!ended(&alice, &call));
}
