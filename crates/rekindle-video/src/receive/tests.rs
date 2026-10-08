use super::*;
use crate::reassembly_state::VideoReassemblyState;
use crate::test_mock::MockDeps;
use rekindle_types::video::Codec;

/// A frame encrypted the way a sender seals it: under its own media key
/// at `index` (plan C7.20). Returns the ciphertext and the key's secret.
fn sealed_by(
    signing_key: &rekindle_secrets::ed25519_dalek::SigningKey,
    index: u64,
    secret: &[u8; 32],
    plaintext: &[u8],
) -> Vec<u8> {
    let frame_key = rekindle_secrets::media_sender_key::video_frame_key(
        secret,
        &signing_key.verifying_key().to_bytes(),
    );
    rekindle_crypto::group::media_key::MediaEncryptionKey::from_bytes(*frame_key, index)
        .encrypt(plaintext)
        .expect("encrypt")
}

/// Build a single-fragment VideoFragment payload signed by the
/// pseudonym derived from `seed` for community `"c1"`, sealed under that
/// sender's media key at index 1, whose key `deps` holds as if pushed.
/// Returns `(sender_hex, payload)`.
fn signed_fragment(
    deps: &MockDeps,
    seed: &[u8; 32],
    forge_signature: bool,
) -> (String, ControlPayload) {
    use rekindle_secrets::derive::{derive_community_pseudonym, sign_with_pseudonym};

    let signing_key = derive_community_pseudonym(seed, "c1");
    let sender_hex = hex::encode(signing_key.verifying_key().to_bytes());
    let secret = [1u8; 32];
    deps.keys
        .install(&sender_hex, 1, zeroize::Zeroizing::new(secret));
    let ciphertext = sealed_by(&signing_key, 1, &secret, b"vp9-keyframe-bytes");

    let mut frag = VideoFragment {
        stream_id: [3u8; 16],
        frame_seq: 1,
        frag_index: 0,
        frag_total: 1,
        keyframe: true,
        codec: Codec::Vp9,
        timestamp: 42,
        key_index: 1,
        payload: ciphertext,
        signature: Vec::new(),
    };
    frag.signature = if forge_signature {
        vec![0u8; 64]
    } else {
        sign_with_pseudonym(
            &signing_key,
            &crate::fragment::fragment_signing_bytes(&frag),
        )
        .to_vec()
    };
    (
        sender_hex,
        ControlPayload::VideoFragment(VideoFragmentPayload {
            channel_id: "11111111111111111111111111111111".into(),
            stream_id: frag.stream_id,
            frame_seq: frag.frame_seq,
            frag_index: frag.frag_index,
            frag_total: frag.frag_total,
            keyframe: frag.keyframe,
            codec: frag.codec,
            timestamp: frag.timestamp,
            key_index: frag.key_index,
            payload: frag.payload,
            signature: frag.signature,
        }),
    )
}

#[test]
fn valid_fragment_signature_reaches_frame_ready() {
    let deps = MockDeps::new();
    let reassembly = VideoReassemblyState::new();
    let (sender_hex, payload) = signed_fragment(&deps, &[9u8; 32], false);
    handle_video_payload(&deps, &reassembly, "c1", &sender_hex, payload, 0);
    let calls = deps.calls.lock();
    assert!(
        calls
            .events
            .iter()
            .any(|e| matches!(e, VideoEvent::FrameReady { .. })),
        "signed + decryptable single-fragment frame must emit FrameReady"
    );
}

#[test]
fn forged_fragment_signature_is_dropped_before_reassembly() {
    let deps = MockDeps::new();
    let reassembly = VideoReassemblyState::new();
    let (sender_hex, payload) = signed_fragment(&deps, &[9u8; 32], true);
    handle_video_payload(&deps, &reassembly, "c1", &sender_hex, payload, 0);
    let calls = deps.calls.lock();
    assert!(
        calls.events.is_empty(),
        "forged fragment signature must never produce an event"
    );
    assert!(
        calls.key_requests.is_empty(),
        "forged fragments must not trigger key requests either"
    );
}

#[test]
fn mismatched_sender_pseudonym_is_dropped() {
    // Valid signature from key A, but the envelope sender is B —
    // a member replaying another member's fragments under their
    // own envelope must be rejected.
    let deps = MockDeps::new();
    let reassembly = VideoReassemblyState::new();
    let (_real_sender, payload) = signed_fragment(&deps, &[9u8; 32], false);
    let other_sender = hex::encode([8u8; 32]);
    handle_video_payload(&deps, &reassembly, "c1", &other_sender, payload, 0);
    assert!(deps.calls.lock().events.is_empty());
}

#[test]
fn a_missing_sender_key_fires_one_debounced_request() {
    use rekindle_secrets::derive::{derive_community_pseudonym, sign_with_pseudonym};

    // The sender seals under its key at index 2, which it has not sent
    // us: one debounced request to that sender for index 2, never a
    // silent drop.
    let deps = MockDeps::new();
    let reassembly = VideoReassemblyState::new();

    let signing_key = derive_community_pseudonym(&[9u8; 32], "c1");
    let sender_hex = hex::encode(signing_key.verifying_key().to_bytes());

    let make_payload = |frame_seq: u32| {
        let mut frag = VideoFragment {
            stream_id: [4u8; 16],
            frame_seq,
            frag_index: 0,
            frag_total: 1,
            keyframe: true,
            codec: Codec::Vp9,
            timestamp: 7,
            key_index: 2,
            payload: sealed_by(&signing_key, 2, &[2u8; 32], b"frame"),
            signature: Vec::new(),
        };
        frag.signature = sign_with_pseudonym(
            &signing_key,
            &crate::fragment::fragment_signing_bytes(&frag),
        )
        .to_vec();
        ControlPayload::VideoFragment(VideoFragmentPayload {
            channel_id: "11111111111111111111111111111111".into(),
            stream_id: frag.stream_id,
            frame_seq: frag.frame_seq,
            frag_index: frag.frag_index,
            frag_total: frag.frag_total,
            keyframe: frag.keyframe,
            codec: frag.codec,
            timestamp: frag.timestamp,
            key_index: frag.key_index,
            payload: frag.payload,
            signature: frag.signature,
        })
    };

    handle_video_payload(&deps, &reassembly, "c1", &sender_hex, make_payload(1), 0);
    // Second frame inside the debounce window: no new request.
    handle_video_payload(
        &deps,
        &reassembly,
        "c1",
        &sender_hex,
        make_payload(2),
        1_000,
    );

    let calls = deps.calls.lock();
    assert_eq!(
        calls.key_requests,
        vec![(
            "c1".to_string(),
            "11111111111111111111111111111111".to_string(),
            sender_hex.clone(),
            2
        )],
        "exactly one debounced request to the sender for its index"
    );
    assert!(
        calls
            .events
            .iter()
            .all(|e| !matches!(e, VideoEvent::FrameReady { .. })),
        "undecryptable frames never reach the frontend"
    );
}

#[test]
fn payload_for_other_channel_is_dropped() {
    // Local user is in ch1; a KeyframeRequest addressed to ch2 must be
    // dropped before any event reaches the frontend.
    let deps = MockDeps::new();
    let reassembly = VideoReassemblyState::new();
    handle_video_payload(
        &deps,
        &reassembly,
        "c1",
        "peer1",
        ControlPayload::KeyframeRequest {
            channel_id: "22222222222222222222222222222222".into(),
            stream_id: [5u8; 16],
        },
        0,
    );
    assert!(
        deps.calls.lock().events.is_empty(),
        "payload for a channel we're not in must not produce events"
    );
}

#[test]
fn payload_with_no_active_session_is_dropped() {
    // Not in any voice/video channel: every video payload is
    // dropped, even with a community MEK cached.
    let deps = MockDeps::in_channel(None);
    let reassembly = VideoReassemblyState::new();
    handle_video_payload(
        &deps,
        &reassembly,
        "c1",
        "peer1",
        ControlPayload::MediaCapabilities {
            channel_id: "11111111111111111111111111111111".into(),
            max_pixel_count: 480 * 854,
            max_fps: 30,
            encode_codecs: vec![rekindle_types::video::Codec::Vp9],
            decode_codecs: vec![rekindle_types::video::Codec::Vp9],
            supports_optimize_for_latency: false,
            supported_scalability_modes: vec![rekindle_types::video::ScalabilityMode::Flat],
        },
        0,
    );
    assert!(
        deps.calls.lock().events.is_empty(),
        "no active session means no video consumption"
    );
}

#[test]
fn fragment_for_other_channel_never_reaches_reassembly() {
    let deps = MockDeps::new();
    let reassembly = VideoReassemblyState::new();
    handle_video_payload(
        &deps,
        &reassembly,
        "c1",
        "peer1",
        ControlPayload::VideoFragment(VideoFragmentPayload {
            channel_id: "22222222222222222222222222222222".into(),
            stream_id: [9u8; 16],
            frame_seq: 1,
            frag_index: 0,
            frag_total: 1,
            keyframe: false,
            codec: Codec::Vp9,
            timestamp: 0,
            key_index: 0,
            payload: vec![0xAB; 64],
            signature: vec![0u8; 64],
        }),
        0,
    );
    assert!(
        deps.calls.lock().events.is_empty(),
        "single-fragment frame for another channel must not be reassembled or emitted"
    );
}

#[test]
fn keyframe_request_resets_stream_and_emits_event() {
    let deps = MockDeps::new();
    let reassembly = VideoReassemblyState::new();
    handle_video_payload(
        &deps,
        &reassembly,
        "c1",
        "peer1",
        ControlPayload::KeyframeRequest {
            channel_id: "11111111111111111111111111111111".into(),
            stream_id: [3u8; 16],
        },
        0,
    );
    let calls = deps.calls.lock();
    assert!(matches!(
        calls.events[0],
        VideoEvent::KeyframeRequest { .. }
    ));
}

#[test]
fn media_capabilities_maps_to_event() {
    let deps = MockDeps::new();
    let reassembly = VideoReassemblyState::new();
    handle_video_payload(
        &deps,
        &reassembly,
        "c1",
        "peer1",
        ControlPayload::MediaCapabilities {
            channel_id: "11111111111111111111111111111111".into(),
            max_pixel_count: 480 * 854,
            max_fps: 30,
            encode_codecs: vec![rekindle_types::video::Codec::H264],
            decode_codecs: vec![
                rekindle_types::video::Codec::Vp9,
                rekindle_types::video::Codec::H264,
            ],
            supports_optimize_for_latency: false,
            supported_scalability_modes: vec![rekindle_types::video::ScalabilityMode::Flat],
        },
        0,
    );
    let calls = deps.calls.lock();
    let VideoEvent::MediaCapabilities {
        ref encode_codecs,
        ref decode_codecs,
        ..
    } = calls.events[0]
    else {
        panic!("expected MediaCapabilities variant");
    };
    // Direction-split lists must map through without being swapped.
    assert_eq!(encode_codecs, &vec![rekindle_types::video::Codec::H264]);
    assert_eq!(
        decode_codecs,
        &vec![
            rekindle_types::video::Codec::Vp9,
            rekindle_types::video::Codec::H264,
        ]
    );
}

#[test]
fn topology_change_higher_lamport_accepted() {
    let deps = MockDeps::new();
    let reassembly = VideoReassemblyState::new();
    handle_video_payload(
        &deps,
        &reassembly,
        "c1",
        "peer1",
        ControlPayload::TopologyChange {
            channel_id: "11111111111111111111111111111111".into(),
            stream_id: [1u8; 16],
            relay_host_pseudonym: None,
            reason: "switch".into(),
            lamport: 5,
        },
        0,
    );
    let calls = deps.calls.lock();
    assert_eq!(calls.events.len(), 1);
    assert!(matches!(calls.events[0], VideoEvent::TopologyChange { .. }));
}

#[test]
fn topology_change_lower_lamport_rejected() {
    let deps = MockDeps::new();
    let reassembly = VideoReassemblyState::new();
    // Accept high lamport first.
    handle_video_payload(
        &deps,
        &reassembly,
        "c1",
        "peer1",
        ControlPayload::TopologyChange {
            channel_id: "11111111111111111111111111111111".into(),
            stream_id: [2u8; 16],
            relay_host_pseudonym: None,
            reason: "switch".into(),
            lamport: 10,
        },
        0,
    );
    let events_after_first = deps.calls.lock().events.len();
    // Lower lamport should be silently dropped.
    handle_video_payload(
        &deps,
        &reassembly,
        "c1",
        "peer1",
        ControlPayload::TopologyChange {
            channel_id: "11111111111111111111111111111111".into(),
            stream_id: [2u8; 16],
            relay_host_pseudonym: None,
            reason: "stale".into(),
            lamport: 5,
        },
        0,
    );
    let events_after_second = deps.calls.lock().events.len();
    assert_eq!(
        events_after_first, events_after_second,
        "stale lamport TopologyChange should be dropped"
    );
}
