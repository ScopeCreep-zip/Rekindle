//! The Host's voice engine state (plan E4.1 S5a1): precedence with the
//! ruling's reasons, leave, local controls, and media reaching the session.

use std::time::Instant;

use rekindle_codec::capnp_codec::SignedWire;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_voice::media_frame::{self, VOICE_TAG};

use super::*;

/// A live session on `scope` over the system clock.
fn live_on(scope: VoiceScope) -> LiveVoice {
    LiveVoice::new(scope, Arc::new(rekindle_utils::SystemClock))
}

fn voice() -> HostVoice {
    HostVoice::new(Arc::new(rekindle_utils::SystemClock))
}

fn room(community: &str) -> VoiceScope {
    VoiceScope::Community {
        community: community.into(),
        channel: "vc".into(),
    }
}

fn name(id: &str) -> String {
    format!("{id}-name")
}

#[test]
fn a_join_installs_the_one_live_session() {
    let v = voice();
    assert_eq!(v.start_refusal(name), None);
    v.begin(live_on(room("c1")), name).unwrap();
    assert_eq!(v.session(), Some(room("c1")));
    // The same scope again is already joined, not a second session.
    v.begin(live_on(room("c1")), name).unwrap();
    assert_eq!(v.session(), Some(room("c1")));
}

#[test]
fn a_second_session_while_live_is_refused_with_the_rulings_reason() {
    let v = voice();
    v.begin(live_on(room("c1")), name).unwrap();
    let refused = v.begin(live_on(room("c2")), name).unwrap_err();
    assert_eq!(refused, "Leave voice in c1-name first.");
    assert_eq!(
        v.start_refusal(name).as_deref(),
        Some("Leave voice in c1-name first.")
    );
    assert_eq!(
        v.session(),
        Some(room("c1")),
        "the first session is never ended implicitly"
    );

    let call = voice();
    call.begin(
        live_on(VoiceScope::Dm {
            peer_key: "p".into(),
        }),
        name,
    )
    .unwrap();
    assert_eq!(
        call.begin(live_on(room("c1")), name).unwrap_err(),
        LEAVE_CALL_FIRST
    );
    let group = voice();
    group
        .begin(
            live_on(VoiceScope::Group {
                group_id: "g".into(),
            }),
            name,
        )
        .unwrap();
    assert_eq!(group.start_refusal(name).as_deref(), Some(LEAVE_CALL_FIRST));
}

#[test]
fn leave_ends_the_session_and_clears_the_refusal() {
    let v = voice();
    v.begin(live_on(room("c1")), name).unwrap();
    assert_eq!(v.end(), Some(room("c1")));
    assert_eq!(v.session(), None);
    assert_eq!(v.start_refusal(name), None);
    assert_eq!(v.end(), None, "leave with nothing live is a no-op");
    v.begin(live_on(room("c2")), name).unwrap();
}

#[test]
fn mute_and_deafen_reach_the_live_session_only() {
    let v = voice();
    assert!(!v.set_muted(true));
    assert!(!v.set_deafened(true));
    v.begin(live_on(room("c1")), name).unwrap();
    assert!(v.set_muted(true));
    assert!(v.set_deafened(true));
    assert_eq!(v.controls(), Some((true, true)));
    assert!(v.set_muted(false));
    assert_eq!(v.controls(), Some((false, true)));
}

fn signed_voice_datagram(key: &SigningKey) -> Vec<u8> {
    let mut packet = VoicePacket {
        sender_key: key.verifying_key().to_bytes().to_vec(),
        sequence: 3,
        timestamp: 60,
        sframe: vec![1, 2, 3],
        sig: Vec::new(),
    };
    packet.sign(key);
    media_frame::sequenced(VOICE_TAG, 9, &packet.encode())
}

#[test]
fn a_media_datagram_reaches_the_live_session() {
    let v = voice();
    let (tx, mut rx) = mpsc::channel(4);
    let mut session = live_on(room("c1"));
    session.packet_tx = Some(tx);
    let liveness = Arc::clone(&session.liveness);
    v.begin(session, name).unwrap();

    let key = SigningKey::from_bytes(&[7; 32]);
    ingest_media(&v, &signed_voice_datagram(&key), Instant::now());

    let packet = rx.try_recv().expect("the receive loop gets the packet");
    assert_eq!(packet.sequence, 3);
    assert!(
        liveness.latest().is_some(),
        "a verified packet is proof of life"
    );
    assert_eq!(v.voice_drops(), 0);
}

#[test]
fn a_media_datagram_with_no_session_is_counted_as_dropped() {
    let v = voice();
    let key = SigningKey::from_bytes(&[7; 32]);
    ingest_media(&v, &signed_voice_datagram(&key), Instant::now());
    assert_eq!(v.voice_drops(), 1);
}
