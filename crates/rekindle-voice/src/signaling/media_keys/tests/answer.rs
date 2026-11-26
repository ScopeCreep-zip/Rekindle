//! The media-key call answers (plan E4.1): a typed ack for an installed
//! key, an empty reply for an accepted request, a NAK for every refusal.

use super::sealing::sealed_to;
use super::*;
use crate::signaling::media_key_answer::{answer_media_key_call, MediaKeyAnswer};
use crate::signaling::media_key_outbox::acknowledges;

/// An installed key is answered with the `VoiceMediaKeyAck` envelope the
/// sender counts as delivered; a bare `ACK` would never count.
#[tokio::test]
async fn an_installed_key_is_answered_with_its_typed_ack() {
    let net = net();
    let alice = Peer::new(&net, 1, false);
    let bob = Peer::new(&net, 2, false);
    bob.roster(&[&alice]).await;
    let payload = key_payload(&alice, &bob.id(), 5, sealed_to(&bob, &[4; 32]));

    let answer = answer_media_key_call(&bob.deps(), &payload).await;
    let MediaKeyAnswer::Ack(reply) = answer else {
        panic!("an installed key is acknowledged, got {answer:?}");
    };
    let reply = rekindle_codec::capnp_envelope::decode_community_envelope(&reply).ok();
    assert!(
        acknowledges(&CommunityEnvelope::control(payload), reply.as_ref()),
        "the reply is the VoiceMediaKeyAck for exactly that key"
    );
}

/// A key from a sender not on our roster is held, not installed: refused.
/// So is one addressed to someone else, and any other payload.
#[tokio::test]
async fn keys_not_installed_are_refused() {
    let net = net();
    let alice = Peer::new(&net, 1, false);
    let bob = Peer::new(&net, 2, false);
    let carol = Peer::new(&net, 3, false);
    let payload = key_payload(&alice, &bob.id(), 5, sealed_to(&bob, &[4; 32]));
    assert_eq!(
        answer_media_key_call(&bob.deps(), &payload).await,
        MediaKeyAnswer::Refused
    );
    bob.roster(&[&alice]).await;
    assert_eq!(
        answer_media_key_call(&carol.deps(), &payload).await,
        MediaKeyAnswer::Refused
    );
    let ack = ControlPayload::VoiceMediaKeyAck {
        community_id: COMMUNITY.into(),
        channel_id: CHANNEL.into(),
        sender: alice.id(),
        recipient: bob.id(),
        key_index: 5,
    };
    assert_eq!(
        answer_media_key_call(&bob.deps(), &ack).await,
        MediaKeyAnswer::Refused
    );
}

/// A request for our key is accepted with an empty reply; one naming
/// another sender is refused.
#[tokio::test]
async fn requests_are_accepted_empty_or_refused() {
    let net = net();
    let alice = Peer::new(&net, 1, false);
    let bob = Peer::new(&net, 2, false);
    let mallory = Peer::new(&net, 4, false);
    alice.roster(&[&bob]).await;
    assert_eq!(
        answer_media_key_call(&alice.deps(), &request(&bob, &alice.id())).await,
        MediaKeyAnswer::Accepted
    );
    assert_eq!(
        answer_media_key_call(&alice.deps(), &request(&bob, &mallory.id())).await,
        MediaKeyAnswer::Refused
    );
    alice.settle().await;
}
