use super::*;
use crate::session_deps::CallMediaKeys;
use ed25519_dalek::SigningKey;
use rekindle_codec::capnp_codec::SignedWire;
use rekindle_secrets::media_sender_key::keyring::ChannelSenderKeys;

/// One side's key source: a call with `call_peer`, and one channel
/// session's sender keys.
struct Keys {
    call_secret: [u8; 32],
    call_peer: String,
    call_sender: Arc<SframeSender>,
    channel: Arc<ChannelSenderKeys>,
}

impl Keys {
    fn new(call_peer: &str) -> Arc<Self> {
        Arc::new(Self {
            call_secret: [7u8; 32],
            call_peer: call_peer.to_string(),
            call_sender: Arc::new(SframeSender::fresh()),
            channel: Arc::new(ChannelSenderKeys::default()),
        })
    }
}

impl MediaKeySource for Keys {
    fn channel_sender_keys(&self, _: &str, _: &str) -> Arc<ChannelSenderKeys> {
        Arc::clone(&self.channel)
    }
    fn call_media(&self, peer: &str) -> Option<CallMediaKeys> {
        (peer == self.call_peer).then(|| CallMediaKeys {
            secret: Zeroizing::new(self.call_secret),
            sender: Arc::clone(&self.call_sender),
        })
    }
}

fn channel() -> MediaScope {
    MediaScope::Channel {
        community_id: "c".into(),
        channel_id: "0a".repeat(16),
    }
}

fn packet(
    signer: &SigningKey,
    sealer: &mut FrameSealer,
    sequence: u32,
    opus: &[u8],
) -> VoicePacket {
    let sender_key = signer.verifying_key().to_bytes().to_vec();
    let sframe = sealer.seal(&sender_key, sequence, 1_000, opus).unwrap();
    let mut p = VoicePacket {
        sender_key,
        sequence,
        timestamp: 1_000,
        sframe,
        sig: Vec::new(),
    };
    p.sign(signer);
    p
}

fn keys(source: &Arc<Keys>, scope: MediaScope) -> Arc<MediaKeys> {
    let source: Arc<dyn MediaKeySource> = source.clone();
    Arc::new(MediaKeys::new(source, scope))
}

/// Give `receiver` the key `sender` sends under now, as a pushed key.
fn share(sender: &Arc<Keys>, sender_hex: &str, receiver: &Arc<Keys>) {
    let key = sender.channel.send_key(std::time::Instant::now());
    receiver.channel.install(sender_hex, key.index, key.secret);
}

#[test]
fn call_frames_open_only_from_the_peer_and_only_once() {
    let alice = SigningKey::from_bytes(&[1u8; 32]);
    let alice_hex = hex::encode(alice.verifying_key().to_bytes());
    let bob_hex = "bb".repeat(32);
    // Each side's scope names the other; both hold the call secret.
    let alice_keys = Keys::new(&bob_hex);
    let bob_keys = Keys::new(&alice_hex);
    let mut sealer = FrameSealer::new(keys(&alice_keys, MediaScope::DirectCall { peer: bob_hex }));
    let mut opener = FrameOpener::new(keys(&bob_keys, MediaScope::DirectCall { peer: alice_hex }));

    let p = packet(&alice, &mut sealer, 1, b"opus");
    assert_eq!(opener.open(&p).unwrap(), b"opus");
    assert_eq!(opener.open(&p), Err(OpenError::Rejected), "replay");

    let mallory = SigningKey::from_bytes(&[2u8; 32]);
    let forged = packet(&mallory, &mut sealer, 2, b"x");
    assert_eq!(opener.open(&forged), Err(OpenError::WrongSender));
}

/// A channel frame opens with the sender's own key once it was pushed
/// to us, and names the index to ask for before then.
#[test]
fn channel_frames_open_under_the_senders_own_key() {
    let alice = SigningKey::from_bytes(&[1u8; 32]);
    let alice_hex = hex::encode(alice.verifying_key().to_bytes());
    let alice_side = Keys::new("unused");
    let bob_side = Keys::new("unused");
    let mut sealer = FrameSealer::new(keys(&alice_side, channel()));
    let p = packet(&alice, &mut sealer, 1, b"hello");

    let mut opener = FrameOpener::new(keys(&bob_side, channel()));
    let index = alice_side.channel.send_key(std::time::Instant::now()).index;
    assert_eq!(
        opener.open(&p),
        Err(OpenError::NoKey {
            needed_index: Some(index & 0xff)
        })
    );
    share(&alice_side, &alice_hex, &bob_side);
    assert_eq!(opener.open(&p).unwrap(), b"hello");
}

/// Another member's key never opens Alice's frames: each sender's key
/// has one writer, so no two members can disagree on it.
#[test]
fn a_frame_does_not_open_under_another_senders_key() {
    let alice = SigningKey::from_bytes(&[1u8; 32]);
    let alice_hex = hex::encode(alice.verifying_key().to_bytes());
    let alice_side = Keys::new("unused");
    let carol_side = Keys::new("unused");
    let bob_side = Keys::new("unused");
    let mut sealer = FrameSealer::new(keys(&alice_side, channel()));
    let p = packet(&alice, &mut sealer, 1, b"x");
    // Bob holds Carol's key filed under Alice: it does not open.
    share(&carol_side, &alice_hex, &bob_side);
    let index = alice_side.channel.send_key(std::time::Instant::now()).index;
    let carol_index = carol_side.channel.send_key(std::time::Instant::now()).index;
    let result = FrameOpener::new(keys(&bob_side, channel())).open(&p);
    if index & 0xff == carol_index & 0xff {
        assert_eq!(result, Err(OpenError::Rejected));
    } else {
        assert!(matches!(result, Err(OpenError::NoKey { .. })));
    }
}

/// A join ratchets the sender's key (plan C7.29): a receiver holding
/// the old step derives the new one, while the joiner, given only the
/// new step, opens nothing sealed before it joined.
#[test]
fn a_joiner_opens_only_frames_sealed_after_its_join() {
    let alice = SigningKey::from_bytes(&[1u8; 32]);
    let alice_hex = hex::encode(alice.verifying_key().to_bytes());
    let alice_side = Keys::new("unused");
    let bob_side = Keys::new("unused");
    let carol_side = Keys::new("unused");
    let mut sealer = FrameSealer::new(keys(&alice_side, channel()));
    share(&alice_side, &alice_hex, &bob_side);
    let before = packet(&alice, &mut sealer, 1, b"before");

    let join = alice_side
        .channel
        .on_join("carol", std::time::Instant::now());
    for key in join.joiner {
        carol_side
            .channel
            .install(&alice_hex, key.index, key.secret);
    }
    let after = packet(&alice, &mut sealer, 2, b"after");

    let mut bob = FrameOpener::new(keys(&bob_side, channel()));
    assert_eq!(bob.open(&before).unwrap(), b"before");
    assert_eq!(bob.open(&after).unwrap(), b"after", "derived forward");

    let mut carol = FrameOpener::new(keys(&carol_side, channel()));
    assert!(carol.open(&before).is_err());
    assert_eq!(carol.open(&after).unwrap(), b"after");
}

#[test]
fn metadata_is_bound() {
    let alice = SigningKey::from_bytes(&[1u8; 32]);
    let alice_hex = hex::encode(alice.verifying_key().to_bytes());
    let source = Keys::new("unused");
    let mut sealer = FrameSealer::new(keys(&source, channel()));
    share(&source, &alice_hex, &source);
    let mut opener = FrameOpener::new(keys(&source, channel()));
    let mut p = packet(&alice, &mut sealer, 1, b"opus");
    p.sequence = 2;
    assert_eq!(opener.open(&p), Err(OpenError::Rejected));
}

/// A rebuilt transport gets a new sealer, but the sender state lives
/// in the session's keys, so the counter continues (RFC 9605 §9.1).
#[test]
fn counter_continues_across_sealer_rebuilds() {
    let source = Keys::new("unused");
    let alice = SigningKey::from_bytes(&[1u8; 32]);
    let mut first = FrameSealer::new(keys(&source, channel()));
    let a = packet(&alice, &mut first, 1, b"a");
    let mut second = FrameSealer::new(keys(&source, channel()));
    let b = packet(&alice, &mut second, 2, b"b");
    let (kid_a, ctr_a, _) = sframe::parse_header(&a.sframe).unwrap();
    let (kid_b, ctr_b, _) = sframe::parse_header(&b.sframe).unwrap();
    assert_eq!(kid_a, kid_b);
    assert_ne!(ctr_a, ctr_b);
}
