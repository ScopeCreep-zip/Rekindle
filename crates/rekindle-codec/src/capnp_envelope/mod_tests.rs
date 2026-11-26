use super::*;

#[test]
fn signed_envelope_roundtrip() {
    let signed = SignedEnvelope {
        message_id: [7u8; 16],
        community_id: "VLD0:abc".into(),
        sender_pseudonym: "deadbeef".into(),
        envelope_bytes: vec![1, 2, 3, 4, 5],
        signature: vec![0u8; 64],
        ttl: 5,
    };
    let bytes = encode_signed_envelope(&signed);
    let back = decode_signed_envelope(&bytes).unwrap();
    assert_eq!(signed.community_id, back.community_id);
    assert_eq!(signed.sender_pseudonym, back.sender_pseudonym);
    assert_eq!(signed.envelope_bytes, back.envelope_bytes);
    assert_eq!(signed.signature, back.signature);
    assert_eq!(signed.ttl, back.ttl);
    assert_eq!(signed.message_id, back.message_id);
}

#[test]
fn typing_indicator_roundtrip() {
    let env = CommunityEnvelope::TypingIndicator {
        channel_tag: [7; 16],
        ttl_ms: 6_000,
        pseudonym_key: "abcd".into(),
    };
    let bytes = encode_community_envelope(&env).unwrap();
    let back = decode_community_envelope(&bytes).unwrap();
    match back {
        CommunityEnvelope::TypingIndicator {
            channel_tag,
            ttl_ms,
            pseudonym_key,
        } => {
            assert_eq!(channel_tag, [7; 16]);
            assert_eq!(ttl_ms, 6_000);
            assert_eq!(pseudonym_key, "abcd");
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn voice_join_handshake_roundtrip() {
    use crate::community::envelope::{ControlPayload, MediaSessionKey};

    // Leg 1 — VoiceJoin with carried display name.
    let env = CommunityEnvelope::control(ControlPayload::VoiceJoin {
        channel_id: "ch_voice".into(),
        route_blob: vec![9, 9, 9],
        display_name: Some("FireStarter92".into()),
        session_key: MediaSessionKey {
            key: vec![5; 32],
            signature: vec![6; 64],
        },
    });
    let bytes = encode_community_envelope(&env).unwrap();
    match decode_community_envelope(&bytes).unwrap().into_control() {
        Some(ControlPayload::VoiceJoin {
            channel_id,
            route_blob,
            display_name,
            session_key,
        }) => {
            assert_eq!(channel_id, "ch_voice");
            assert_eq!(route_blob, vec![9, 9, 9]);
            assert_eq!(display_name.as_deref(), Some("FireStarter92"));
            assert_eq!(session_key.key, vec![5; 32]);
            assert_eq!(session_key.signature, vec![6; 64]);
        }
        _ => panic!("wrong variant"),
    }

    // Leg 2 — VoiceJoinAck carries acker identity + route.
    let env = CommunityEnvelope::control(ControlPayload::VoiceJoinAck {
        channel_id: "ch_voice".into(),
        joiner_pseudonym: "joiner_hex".into(),
        display_name: None,
        route_blob: vec![4, 2],
        session_key: MediaSessionKey {
            key: vec![7; 32],
            signature: vec![8; 64],
        },
    });
    let bytes = encode_community_envelope(&env).unwrap();
    match decode_community_envelope(&bytes).unwrap().into_control() {
        Some(ControlPayload::VoiceJoinAck {
            channel_id,
            joiner_pseudonym,
            display_name,
            route_blob,
            session_key,
        }) => {
            assert_eq!(channel_id, "ch_voice");
            assert_eq!(joiner_pseudonym, "joiner_hex");
            assert_eq!(display_name, None, "empty wire text decodes to None");
            assert_eq!(route_blob, vec![4, 2]);
            assert_eq!(session_key.key, vec![7; 32]);
            assert_eq!(session_key.signature, vec![8; 64]);
        }
        _ => panic!("wrong variant"),
    }

    // Leg 3 — VoiceJoinConfirmed.
    let env = CommunityEnvelope::control(ControlPayload::VoiceJoinConfirmed {
        channel_id: "ch_voice".into(),
    });
    let bytes = encode_community_envelope(&env).unwrap();
    match decode_community_envelope(&bytes).unwrap().into_control() {
        Some(ControlPayload::VoiceJoinConfirmed { channel_id }) => {
            assert_eq!(channel_id, "ch_voice");
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn voice_roster_entry_display_name_roundtrip() {
    use crate::community::envelope::{ControlPayload, MediaSessionKey, VoiceRosterEntry};

    let env = CommunityEnvelope::control(ControlPayload::VoiceRoster {
        channel_id: "ch_voice".into(),
        participants: vec![
            VoiceRosterEntry {
                pseudonym_key: "alice_hex".into(),
                route_blob: vec![1],
                muted: true,
                deafened: false,
                display_name: Some("Alice".into()),
                session_key: MediaSessionKey {
                    key: vec![3; 32],
                    signature: vec![4; 64],
                },
            },
            VoiceRosterEntry {
                pseudonym_key: "bob_hex".into(),
                route_blob: vec![2],
                muted: false,
                deafened: false,
                display_name: None,
                session_key: MediaSessionKey::default(),
            },
        ],
    });
    let bytes = encode_community_envelope(&env).unwrap();
    match decode_community_envelope(&bytes).unwrap().into_control() {
        Some(ControlPayload::VoiceRoster { participants, .. }) => {
            assert_eq!(participants.len(), 2);
            assert_eq!(participants[0].display_name.as_deref(), Some("Alice"));
            assert!(participants[0].muted);
            assert_eq!(participants[1].display_name, None);
            assert_eq!(participants[0].session_key.key, vec![3; 32]);
            assert_eq!(participants[0].session_key.signature, vec![4; 64]);
            assert_eq!(participants[1].session_key, MediaSessionKey::default());
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn watch_relay_roundtrip() {
    let env = CommunityEnvelope::WatchRelay {
        record_key: "VLD0:xyz".into(),
        subkey: 12,
        content_hash: "blake3:abc".into(),
        observer_pseudonym: "obsvr".into(),
    };
    let bytes = encode_community_envelope(&env).unwrap();
    let back = decode_community_envelope(&bytes).unwrap();
    match back {
        CommunityEnvelope::WatchRelay {
            record_key,
            subkey,
            content_hash,
            observer_pseudonym,
        } => {
            assert_eq!(record_key, "VLD0:xyz");
            assert_eq!(subkey, 12);
            assert_eq!(content_hash, "blake3:abc");
            assert_eq!(observer_pseudonym, "obsvr");
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn try_decode_returns_ok_for_known_variant() {
    let env = CommunityEnvelope::TypingIndicator {
        channel_tag: [1; 16],
        ttl_ms: 0,
        pseudonym_key: "abc".into(),
    };
    let bytes = encode_community_envelope(&env).unwrap();
    let back = try_decode_community_envelope(&bytes).unwrap();
    assert!(back.is_some());
}

/// Forward-compat contract: malformed bytes return `Err`, distinct
/// from the `Ok(None)` returned by `try_decode_community_envelope`
/// when a known-schema reader sees an unknown union discriminant
/// (i.e. a peer running a newer build that added a future
/// variant). Gossip relays use the latter to forward bytes intact;
/// the former drops + does not forward. Synthesising an actual
/// unknown-discriminant message requires low-level Cap'n Proto
/// wire-format construction; that path is covered by the integration
/// path in `services/veilid/app_message.rs::handle_gossip_envelope`
/// (verifies signature → decrements TTL → forwards even on
/// `Ok(None)`).
#[test]
fn try_decode_distinguishes_malformed_from_unknown() {
    let garbage = vec![0xff_u8; 128];
    let result = try_decode_community_envelope(&garbage);
    assert!(
        matches!(result, Err(CodecError::Deserialization(_))),
        "malformed bytes should return Err(Deserialization), not Ok(None) — \
             that's reserved for known-schema-with-unknown-discriminant. Got: {result:?}"
    );
}
