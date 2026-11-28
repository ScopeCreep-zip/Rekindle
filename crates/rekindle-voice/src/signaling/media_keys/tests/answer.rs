use rekindle_codec::community::envelope::ControlPayload;

use super::sealing::sealed_to;
use super::{key_payload, net, on_peer_added, request, Peer, CHANNEL, COMMUNITY};
use crate::signaling::{answer_media_key_call, MediaKeyAnswer};

#[tokio::test]
async fn an_installed_key_is_answered_with_its_typed_ack() {
    let net = net();
    let alice = Peer::new(&net, 1, false);
    let bob = Peer::new(&net, 2, false);
    bob.roster(&[&alice]).await;
    let payload = key_payload(&alice, &bob.id(), 5, sealed_to(&bob, &[4; 32]));

    let answer = answer_media_key_call(&bob.deps(), &payload).await;
    assert_eq!(alice.held_by(&bob, 5), Some([4; 32]));
    assert!(matches!(answer, MediaKeyAnswer::Installed(ack) if payload.acknowledged_by(&ack)));
}

#[tokio::test]
async fn keys_not_installed_are_refused() {
    let net = net();
    let alice = Peer::new(&net, 1, false);
    let bob = Peer::new(&net, 2, false);
    let carol = Peer::new(&net, 3, false);
    let payload = key_payload(&alice, &bob.id(), 5, sealed_to(&bob, &[4; 32]));
    assert!(matches!(
        answer_media_key_call(&bob.deps(), &payload).await,
        MediaKeyAnswer::Refused
    ));
    assert_eq!(alice.held_by(&bob, 5), None);
    assert!(matches!(
        answer_media_key_call(&carol.deps(), &payload).await,
        MediaKeyAnswer::Refused
    ));
    let ack = ControlPayload::VoiceMediaKeyAck {
        community_id: COMMUNITY.into(),
        channel_id: CHANNEL.into(),
        sender: alice.id(),
        recipient: bob.id(),
        key_index: 5,
    };
    assert!(matches!(
        answer_media_key_call(&bob.deps(), &ack).await,
        MediaKeyAnswer::Refused
    ));
}

#[tokio::test]
async fn requests_are_accepted_empty_or_refused() {
    let net = net();
    let alice = Peer::new(&net, 1, false);
    let bob = Peer::new(&net, 2, false);
    let outsider = Peer::new(&net, 4, false);
    alice.roster(&[&bob]).await;
    bob.roster(&[&alice]).await;
    assert!(matches!(
        answer_media_key_call(&alice.deps(), &request(&bob, &alice.id())).await,
        MediaKeyAnswer::Refused
    ));
    on_peer_added(
        &alice.deps(),
        COMMUNITY,
        CHANNEL,
        &alice.transport,
        &bob.id(),
    )
    .await;
    alice.settle().await;
    alice.take_calls();
    assert!(matches!(
        answer_media_key_call(&alice.deps(), &request(&bob, &alice.id())).await,
        MediaKeyAnswer::Requested
    ));
    assert!(matches!(
        answer_media_key_call(&alice.deps(), &request(&outsider, &alice.id())).await,
        MediaKeyAnswer::Refused
    ));
    alice.settle().await;
    let calls = alice.take_calls();
    assert!(!calls.is_empty());
    assert!(calls.iter().all(|(recipient, _)| *recipient == bob.id()));
}
