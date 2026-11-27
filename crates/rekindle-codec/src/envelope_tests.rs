use super::*;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::{ChannelId, PseudonymKey};

#[test]
fn sign_verify_roundtrip() {
    let secret = [42u8; 32];
    let community_id = "VLD0:test_community";
    let payload = GovernanceEntry::CommunityMeta {
        name: Some("Test".into()),
        description: None,
        icon_hash: None,
        banner_hash: None,
    };

    let signed =
        build_signed_envelope(&secret, community_id, [3u8; MESSAGE_ID_LEN], &payload).unwrap();
    assert!(verify_signed_envelope(&signed).is_ok());
}

#[test]
fn tampered_payload_fails_verify() {
    let secret = [42u8; 32];
    let payload = GovernanceEntry::MEKGenerationBump {
        generation: 1,
        trigger_departed: PseudonymKey([0; 32]),
        cascade_skipped: vec![],
    };

    let mut signed = build_signed_envelope(&secret, "c", [3u8; MESSAGE_ID_LEN], &payload).unwrap();
    // Tamper with the payload
    signed.envelope_bytes[0] ^= 0xFF;
    assert!(verify_signed_envelope(&signed).is_err());
}

/// The id is signed: a relay (or attacker) that re-stamps a seen
/// message with a fresh id, to slip it past every dedup cache, breaks
/// the signature.
#[test]
fn restamped_message_id_fails_verify() {
    let payload = GovernanceEntry::MEKGenerationBump {
        generation: 1,
        trigger_departed: PseudonymKey([0; 32]),
        cascade_skipped: vec![],
    };
    let mut signed =
        build_signed_envelope(&[42u8; 32], "c", [3u8; MESSAGE_ID_LEN], &payload).unwrap();
    assert!(verify_signed_envelope(&signed).is_ok());
    signed.message_id[0] ^= 0xFF;
    assert!(verify_signed_envelope(&signed).is_err());
}

#[test]
fn wrong_sender_fails_verify() {
    let secret = [42u8; 32];
    let payload = GovernanceEntry::MEKGenerationBump {
        generation: 1,
        trigger_departed: PseudonymKey([0; 32]),
        cascade_skipped: vec![],
    };

    let mut signed = build_signed_envelope(&secret, "c", [3u8; MESSAGE_ID_LEN], &payload).unwrap();
    // Replace sender with a different pseudonym
    let wrong_key = rekindle_secrets::derive::derive_community_pseudonym(&[99u8; 32], "c");
    signed.sender_pseudonym = hex::encode(wrong_key.verifying_key().to_bytes());
    assert!(verify_signed_envelope(&signed).is_err());
}

#[test]
fn deserialize_payload_roundtrip() {
    let secret = [42u8; 32];
    let payload = GovernanceEntry::ChannelCreated {
        channel_id: ChannelId([1u8; 16]),
        name: "general".into(),
        channel_type: "text".into(),
        category_id: None,
        position: 0,
        parent_voice_channel_id: None,
    };

    let signed = build_signed_envelope(&secret, "c", [3u8; MESSAGE_ID_LEN], &payload).unwrap();
    let deserialized: GovernanceEntry = deserialize_payload(&signed).unwrap();
    assert_eq!(payload, deserialized);
}

#[test]
fn forward_decrements_ttl() {
    let secret = [42u8; 32];
    let payload = GovernanceEntry::MEKGenerationBump {
        generation: 1,
        trigger_departed: PseudonymKey([0; 32]),
        cascade_skipped: vec![],
    };
    let signed = build_signed_envelope(&secret, "c", [3u8; MESSAGE_ID_LEN], &payload).unwrap();
    assert_eq!(signed.ttl, DEFAULT_TTL);

    let fwd = forward_envelope(&signed).unwrap();
    assert_eq!(fwd.ttl, DEFAULT_TTL - 1);
    // Signature and message id unchanged: a relay forwards one message.
    assert_eq!(fwd.signature, signed.signature);
    assert_eq!(fwd.message_id, signed.message_id);
}

#[test]
fn forward_at_zero_returns_none() {
    let mut signed = build_signed_envelope(
        &[1u8; 32],
        "c",
        [3u8; MESSAGE_ID_LEN],
        &GovernanceEntry::MEKGenerationBump {
            generation: 1,
            trigger_departed: PseudonymKey([0; 32]),
            cascade_skipped: vec![],
        },
    )
    .unwrap();
    signed.ttl = 0;
    assert!(forward_envelope(&signed).is_none());
}
