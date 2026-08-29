use ed25519_dalek::{SigningKey, VerifyingKey};
use rekindle_types::video::{Codec, ScalabilityMode};

use super::*;
use crate::capnp_envelope::{encode_community_envelope, encode_signed_envelope};

#[test]
fn sign_and_verify_roundtrip() {
    let secret = [42u8; 32];
    let signing_key = SigningKey::from_bytes(&secret);
    let verifying_key = VerifyingKey::from(&signing_key);
    let pseudonym_hex = hex::encode(verifying_key.to_bytes());

    let envelope = CommunityEnvelope::TypingIndicator {
        channel_id: "ch_01".into(),
        pseudonym_key: pseudonym_hex.clone(),
    };
    // Wire-format encoding matches the live gossip path (Cap'n
    // Proto, not JSON) so the signature applies to bytes that the
    // production code actually sends.
    let envelope_bytes = encode_community_envelope(&envelope).unwrap();

    let signed = sign_envelope(
        &signing_key,
        "community_abc",
        &pseudonym_hex,
        &envelope_bytes,
    );

    assert!(verify_envelope(&signed).is_ok());
}

#[test]
fn verify_rejects_tampered_data() {
    let secret = [42u8; 32];
    let signing_key = SigningKey::from_bytes(&secret);
    let verifying_key = VerifyingKey::from(&signing_key);
    let pseudonym_hex = hex::encode(verifying_key.to_bytes());

    let envelope_bytes = b"original data";
    let mut signed = sign_envelope(
        &signing_key,
        "community_abc",
        &pseudonym_hex,
        envelope_bytes,
    );

    // Tamper with the data
    signed.envelope_bytes = b"tampered data".to_vec();

    assert!(verify_envelope(&signed).is_err());
}

#[test]
fn verify_rejects_wrong_key() {
    let secret1 = [42u8; 32];
    let secret2 = [99u8; 32];
    let signing_key = SigningKey::from_bytes(&secret1);
    let wrong_verifying = VerifyingKey::from(&SigningKey::from_bytes(&secret2));
    let wrong_hex = hex::encode(wrong_verifying.to_bytes());

    let envelope_bytes = b"test data";
    let mut signed = sign_envelope(&signing_key, "community_abc", "placeholder", envelope_bytes);

    // Replace sender pseudonym with wrong key
    signed.sender_pseudonym = wrong_hex;

    assert!(verify_envelope(&signed).is_err());
}

/// Phase 2 — the per-fragment codec tag (RTP payload-type analog)
/// must survive the Cap'n Proto wire form for BOTH fragment
/// variants. Non-VP9 codecs chosen so a write/read arm that
/// silently defaulted to `vp9 @0` fails the assert.
#[test]
fn envelope_video_fragment_codec_capnp_roundtrip() {
    let data = CommunityEnvelope::Control(ControlPayload::VideoFragment(VideoFragmentPayload {
        channel_id: "ch_video".into(),
        stream_id: [0xAB; 16],
        frame_seq: 7,
        frag_index: 0,
        frag_total: 2,
        keyframe: true,
        codec: Codec::H264,
        timestamp: 1234,
        mek_generation: 7,
        payload: vec![1, 2, 3],
        signature: vec![9; 64],
    }));
    let bytes = encode_community_envelope(&data).unwrap();
    let back = crate::capnp_envelope::decode_community_envelope(&bytes).unwrap();
    match back {
        CommunityEnvelope::Control(ControlPayload::VideoFragment(VideoFragmentPayload {
            codec,
            keyframe,
            frame_seq,
            ..
        })) => {
            assert_eq!(codec, Codec::H264);
            assert!(keyframe);
            assert_eq!(frame_seq, 7);
        }
        other => panic!("wrong variant after capnp round-trip: {other:?}"),
    }

    let parity = CommunityEnvelope::Control(ControlPayload::VideoParityFragment(
        VideoParityFragmentPayload {
            channel_id: "ch_video".into(),
            stream_id: [0xCD; 16],
            frame_seq: 8,
            parity_index: 1,
            parity_total: 2,
            data_count: 4,
            codec: Codec::Vp8,
            frame_len: 4096,
            timestamp: 5678,
            mek_generation: 7,
            payload: vec![4, 5, 6],
            signature: vec![7; 64],
        },
    ));
    let bytes = encode_community_envelope(&parity).unwrap();
    let back = crate::capnp_envelope::decode_community_envelope(&bytes).unwrap();
    match back {
        CommunityEnvelope::Control(ControlPayload::VideoParityFragment(
            VideoParityFragmentPayload {
                codec,
                data_count,
                frame_len,
                ..
            },
        )) => {
            assert_eq!(codec, Codec::Vp8);
            assert_eq!(data_count, 4);
            assert_eq!(frame_len, 4096);
        }
        other => panic!("wrong variant after capnp round-trip: {other:?}"),
    }
}

/// Phase 3 schema break — `MediaCapabilities` carries direction-split
/// `encode_codecs` + `decode_codecs` typed lists plus
/// `supports_optimize_for_latency`. Round-trip through the Cap'n
/// Proto wire form and assert every field comes back untouched —
/// asymmetric lists chosen so a swapped encode/decode mapping fails.
#[test]
fn envelope_media_capabilities_capnp_roundtrip() {
    let inner = ControlPayload::MediaCapabilities {
        channel_id: "ch_42".into(),
        max_pixel_count: 1280 * 720,
        max_fps: 30,
        encode_codecs: vec![Codec::H264],
        decode_codecs: vec![Codec::Vp9, Codec::Vp8, Codec::H264],
        supports_optimize_for_latency: true,
        supported_scalability_modes: vec![ScalabilityMode::Flat, ScalabilityMode::L1T2],
    };
    let envelope = CommunityEnvelope::Control(inner);
    let bytes = encode_community_envelope(&envelope).unwrap();
    let back = crate::capnp_envelope::decode_community_envelope(&bytes).unwrap();
    match back {
        CommunityEnvelope::Control(ControlPayload::MediaCapabilities {
            channel_id,
            max_pixel_count,
            max_fps,
            encode_codecs,
            decode_codecs,
            supports_optimize_for_latency,
            supported_scalability_modes,
        }) => {
            assert_eq!(channel_id, "ch_42");
            assert_eq!(max_pixel_count, 1280 * 720);
            assert_eq!(max_fps, 30);
            assert_eq!(encode_codecs, vec![Codec::H264]);
            assert_eq!(decode_codecs, vec![Codec::Vp9, Codec::Vp8, Codec::H264]);
            assert!(supports_optimize_for_latency);
            assert_eq!(
                supported_scalability_modes,
                vec![ScalabilityMode::Flat, ScalabilityMode::L1T2]
            );
        }
        other => panic!("wrong variant after capnp round-trip: {other:?}"),
    }
}

/// Phase A — sign + verify a `MediaCapabilities` envelope to confirm
/// the typed shape rides the same signed-envelope path as the rest
/// of the gossip surface.
#[test]
fn envelope_media_capabilities_sign_and_verify() {
    let signing_key = SigningKey::from_bytes(&[7u8; 32]);
    let verifying_key = VerifyingKey::from(&signing_key);
    let pseudo = hex::encode(verifying_key.to_bytes());
    let envelope = CommunityEnvelope::Control(ControlPayload::MediaCapabilities {
        channel_id: "ch_42".into(),
        max_pixel_count: 854 * 480,
        max_fps: 15,
        encode_codecs: vec![Codec::Vp9],
        decode_codecs: vec![Codec::Vp9],
        supports_optimize_for_latency: false,
        supported_scalability_modes: vec![ScalabilityMode::Flat],
    });
    let bytes = encode_community_envelope(&envelope).unwrap();
    let signed = sign_envelope(&signing_key, "community_xyz", &pseudo, &bytes);
    assert!(verify_envelope(&signed).is_ok());
}

#[test]
fn envelope_message_notification_capnp_roundtrip() {
    let envelope = CommunityEnvelope::MessageNotification {
        channel_id: "ch_01".into(),
        message_id: "msg_abc".into(),
        author_pseudonym: "pseudo_123".into(),
        subkey_index: 7,
        lamport_ts: 42,
        sequence: 7,
        content_hash: "abc123".into(),
        timestamp: 1_234_567_890,
    };
    let bytes = encode_community_envelope(&envelope).unwrap();
    let back = crate::capnp_envelope::decode_community_envelope(&bytes).unwrap();
    match back {
        CommunityEnvelope::MessageNotification { channel_id, .. } => {
            assert_eq!(channel_id, "ch_01");
        }
        _ => panic!("wrong variant"),
    }
}

/// Regression guard: MessageNotification must NEVER contain a "ciphertext" field
/// in the typed Rust enum. Gossip carries the cargo manifest (metadata), not the
/// cargo (ciphertext). Ciphertext exists only on DHT storage nodes (5 replicas),
/// not across the entire gossip fan-out graph.
#[test]
fn message_notification_contains_no_ciphertext() {
    let envelope = CommunityEnvelope::MessageNotification {
        channel_id: "ch_01".into(),
        message_id: "msg_abc".into(),
        author_pseudonym: "pseudo_123".into(),
        subkey_index: 7,
        lamport_ts: 42,
        sequence: 7,
        content_hash: "abc123def456".into(),
        timestamp: 1_234_567_890,
    };
    // Debug-format inspection: `Debug` derives the field names the
    // type really has. If a `ciphertext` field is ever added to
    // `MessageNotification`, this regression catches it.
    let debug = format!("{envelope:?}");
    assert!(
        !debug.contains("ciphertext"),
        "MessageNotification must NOT contain ciphertext — gossip carries \
         notifications only. Got: {debug}"
    );
}

/// Regression guard: `MessageNotification` packs compactly on the
/// wire. Cap'n Proto packed format keeps this notification well
/// under the Veilid 32 KB `app_message` limit; if ciphertext
/// sneaks in, this limit fails immediately.
#[test]
fn message_notification_payload_stays_compact() {
    let envelope = CommunityEnvelope::MessageNotification {
        channel_id: "ch01".into(),
        message_id: "m01".into(),
        author_pseudonym: "p01".into(),
        subkey_index: 7,
        lamport_ts: 42,
        sequence: 3,
        content_hash: "abc123".into(),
        timestamp: 1_234_567_890,
    };
    let bytes = encode_community_envelope(&envelope).unwrap();
    assert!(
        bytes.len() < 200,
        "MessageNotification should be compact (< 200 bytes packed), was {} bytes. \
         If this fails, check if ciphertext or large fields were added.",
        bytes.len()
    );
}

#[test]
fn control_payload_capnp_roundtrip() {
    let payload = ControlPayload::MemberLeave {
        pseudonym_key: "abc123".into(),
    };
    let bytes = encode_community_envelope(&CommunityEnvelope::Control(payload)).unwrap();
    let back = crate::capnp_envelope::decode_community_envelope(&bytes).unwrap();
    match back {
        CommunityEnvelope::Control(ControlPayload::MemberLeave { pseudonym_key }) => {
            assert_eq!(pseudonym_key, "abc123");
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn mek_transfer_ack_capnp_roundtrip() {
    // P1.3 — verify the new MekTransferAck variant survives Cap'n
    // Proto encode/decode with all fields preserved including the
    // optional channel_id (hasChannelId boolean toggle).
    let payload = ControlPayload::MekTransferAck(MekTransferAckPayload {
        community_id: "veilid:abc".into(),
        channel_id: Some("ch_42".into()),
        generation: 17,
        requester_pseudonym: "deadbeef".repeat(4), // 32-hex-char pseudonym
    });
    let bytes = encode_community_envelope(&CommunityEnvelope::Control(payload)).unwrap();
    let back = crate::capnp_envelope::decode_community_envelope(&bytes).unwrap();
    match back {
        CommunityEnvelope::Control(ControlPayload::MekTransferAck(MekTransferAckPayload {
            community_id,
            channel_id,
            generation,
            requester_pseudonym,
        })) => {
            assert_eq!(community_id, "veilid:abc");
            assert_eq!(channel_id.as_deref(), Some("ch_42"));
            assert_eq!(generation, 17);
            assert_eq!(requester_pseudonym, "deadbeef".repeat(4));
        }
        _ => panic!("wrong variant"),
    }

    // Also verify the channel_id == None path round-trips cleanly
    // — this is the community-wide MEK case.
    let payload_none = ControlPayload::MekTransferAck(MekTransferAckPayload {
        community_id: "veilid:xyz".into(),
        channel_id: None,
        generation: 99,
        requester_pseudonym: "cafebabe".repeat(4),
    });
    let bytes = encode_community_envelope(&CommunityEnvelope::Control(payload_none)).unwrap();
    let back = crate::capnp_envelope::decode_community_envelope(&bytes).unwrap();
    match back {
        CommunityEnvelope::Control(ControlPayload::MekTransferAck(MekTransferAckPayload {
            channel_id,
            generation,
            ..
        })) => {
            assert!(channel_id.is_none(), "None channel_id must round-trip");
            assert_eq!(generation, 99);
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn request_segment_expansion_capnp_roundtrip() {
    // P4.3 — RequestSegmentExpansion variant must round-trip
    // cleanly through the Cap'n Proto codec.
    let payload = ControlPayload::RequestSegmentExpansion {
        community_id: "veilid:plate".into(),
        requester_pseudonym: "feedface".repeat(4),
        full_segment_index: 0,
    };
    let bytes = encode_community_envelope(&CommunityEnvelope::Control(payload)).unwrap();
    let back = crate::capnp_envelope::decode_community_envelope(&bytes).unwrap();
    match back {
        CommunityEnvelope::Control(ControlPayload::RequestSegmentExpansion {
            community_id,
            requester_pseudonym,
            full_segment_index,
        }) => {
            assert_eq!(community_id, "veilid:plate");
            assert_eq!(requester_pseudonym, "feedface".repeat(4));
            assert_eq!(full_segment_index, 0);
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn signed_envelope_capnp_roundtrip() {
    let signed = SignedEnvelope {
        community_id: "comm_01".into(),
        sender_pseudonym: "abc123".into(),
        envelope_bytes: vec![1, 2, 3],
        signature: vec![0u8; 64],
        ttl: 5,
    };
    let bytes = encode_signed_envelope(&signed);
    let back = crate::capnp_envelope::decode_signed_envelope(&bytes).unwrap();
    assert_eq!(back.community_id, "comm_01");
}
