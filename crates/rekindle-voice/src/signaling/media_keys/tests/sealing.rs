//! Sealing media keys to session keys (plan C7.29 G2): signatures bind
//! sender, recipient and index, and a key sealed to an ended session
//! stays closed.

use super::*;

/// `secret` sealed to `recipient`'s current session key under index 5.
fn sealed_to(recipient: &Peer, secret: &[u8; 32]) -> Vec<u8> {
    let aad = seal_aad(CHANNEL, &recipient.id(), 5);
    session::seal(&recipient.sender_keys().session_public(), &aad, secret).unwrap()
}

/// A key addressed to someone else, relabelled with another index, or
/// claimed by another sender fails its signature and is neither installed
/// nor acknowledged.
#[tokio::test]
async fn misdirected_or_rebound_keys_are_refused() {
    let net = net();
    let alice = Peer::new(&net, 1, false);
    let bob = Peer::new(&net, 2, false);
    let carol = Peer::new(&net, 3, false);
    let genuine = key_payload(&alice, &bob.id(), 5, sealed_to(&bob, &[4; 32]));
    let relabel = |f: &dyn Fn(&mut ControlPayload)| {
        let mut p = genuine.clone();
        f(&mut p);
        p
    };

    // Sealed and signed for Bob, addressed to Carol.
    let to_carol = relabel(&|p| {
        if let ControlPayload::VoiceMediaKey { recipient, .. } = p {
            *recipient = carol.id();
        }
    });
    assert!(handle_voice_media_key(&carol.deps(), &to_carol).is_none());
    // Relabelled index: the signature no longer verifies.
    let rebound = relabel(&|p| {
        if let ControlPayload::VoiceMediaKey { key_index, .. } = p {
            *key_index = 6;
        }
    });
    assert!(handle_voice_media_key(&bob.deps(), &rebound).is_none());
    // Claimed to be from Carol: Alice's signature is not Carol's.
    let forged = relabel(&|p| {
        if let ControlPayload::VoiceMediaKey { sender, .. } = p {
            *sender = carol.id();
        }
    });
    assert!(handle_voice_media_key(&bob.deps(), &forged).is_none());
    assert!(bob.sender_keys().secret_at(&carol.id(), 5).is_err());

    // The genuine one installs and is acknowledged for exactly that key.
    let ack = handle_voice_media_key(&bob.deps(), &genuine).unwrap();
    assert!(acknowledges(
        &CommunityEnvelope::Control(genuine),
        Some(&CommunityEnvelope::Control(ack))
    ));
    assert_eq!(alice.held_by(&bob, 5), Some([4; 32]));
}

/// Once Bob's channel session ends, a recorded `VoiceMediaKey` sealed to
/// him does not open, though every pseudonym secret is known here: the
/// session key it was sealed to is gone (plan C7.29 G2, RFC 9180 §9.1).
#[tokio::test]
async fn a_recorded_key_does_not_open_after_the_session_ends() {
    let net = net();
    let alice = Peer::new(&net, 1, false);
    let bob = Peer::new(&net, 2, false);
    let recorded = key_payload(&alice, &bob.id(), 5, sealed_to(&bob, &[4; 32]));
    bob.keys.end(COMMUNITY, CHANNEL);
    assert!(handle_voice_media_key(&bob.deps(), &recorded).is_none());
    assert_eq!(alice.held_by(&bob, 5), None);
}

/// A delivery waits for the recipient's session key: nothing is sent
/// until its join or ack announced one.
#[tokio::test(start_paused = true)]
async fn nothing_is_sealed_before_the_recipient_announced_a_session_key() {
    let net = net();
    let alice = Peer::new(&net, 1, false);
    let bob = Peer::new(&net, 2, false);
    alice
        .transport
        .lock()
        .await
        .add_peer(&bob.id(), &bob.route, None);
    on_peer_added(
        &alice.deps(),
        COMMUNITY,
        CHANNEL,
        &alice.transport,
        &bob.id(),
    )
    .await;
    alice.settle().await;
    assert!(alice.take_calls().is_empty());
}
