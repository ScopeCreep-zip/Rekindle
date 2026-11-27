use super::*;

#[test]
fn presence_serde_roundtrip() {
    let presence = MemberPresence {
        pseudonym_key: PseudonymKey([0xAB; 32]),
        display_name: Some("FireStarter92".into()),
        status: "online".into(),
        route_blob: vec![1, 2, 3],
        last_heartbeat: 1_710_000_000,
        ..Default::default()
    };
    let json = serde_json::to_string(&presence).unwrap();
    let back: MemberPresence = serde_json::from_str(&json).unwrap();
    assert_eq!(presence, back);
}

#[test]
fn encrypted_history_ranges_roundtrip() {
    // W11.2 — wire format must survive serde and the signature
    // path. Build a MemberPresence with the encrypted field
    // populated and verify serialize → deserialize produces an
    // identical struct (signing_bytes consistency).
    let presence = MemberPresence {
        pseudonym_key: PseudonymKey([0xCD; 32]),
        display_name: Some("Tester".into()),
        history_ranges_encrypted: Some(EncryptedHistoryRanges {
            mek_generation: 7,
            ciphertext: vec![0xDE, 0xAD, 0xBE, 0xEF, 0x10, 0x20, 0x30],
        }),
        ..Default::default()
    };
    let json = serde_json::to_string(&presence).unwrap();
    let back: MemberPresence = serde_json::from_str(&json).unwrap();
    assert_eq!(presence, back);
    // The same input produces identical signing bytes → signature
    // verification is stable across encode/decode.
    assert_eq!(presence.signing_bytes(), back.signing_bytes());
}

#[test]
fn encrypted_history_ranges_omitted_when_none() {
    // `skip_serializing_if = "Option::is_none"` keeps the JSON
    // small for the common case (most presence rows don't carry
    // history ads). Confirm the field literally doesn't appear in
    // the wire form when None.
    let presence = MemberPresence {
        pseudonym_key: PseudonymKey([0; 32]),
        ..Default::default()
    };
    let json = serde_json::to_string(&presence).unwrap();
    assert!(
        !json.contains("historyRangesEncrypted"),
        "absent field must not serialize"
    );
}

#[test]
fn default_presence_is_online() {
    let p = MemberPresence::default();
    assert_eq!(p.status, "online");
    assert!(!p.in_call);
    assert!(p.route_blob.is_empty());
    assert_eq!(p.session.status, SessionStatus::Online);
    assert!(p.session.location.is_none());
    assert!(p.session_extras_encrypted.is_none());
}

#[test]
fn session_status_wire_strings() {
    assert_eq!(SessionStatus::Online.as_wire_str(), "online");
    assert_eq!(SessionStatus::Away.as_wire_str(), "away");
    assert_eq!(SessionStatus::Busy.as_wire_str(), "busy");
    assert_eq!(SessionStatus::Offline.as_wire_str(), "offline");
    // Invisible must never leak as a distinct wire status.
    assert_eq!(SessionStatus::Invisible.as_wire_str(), "offline");
}

#[test]
fn session_serde_roundtrip_and_plaintext_strips_extras() {
    // The plaintext `session` field must not serialise location /
    // activity — they ride encrypted. With None they're omitted.
    let session = MemberSession {
        status: SessionStatus::Away,
        location: None,
        activity: None,
        last_active: 42,
    };
    let json = serde_json::to_string(&session).unwrap();
    assert!(!json.contains("location"));
    assert!(!json.contains("activity"));
    let back: MemberSession = serde_json::from_str(&json).unwrap();
    assert_eq!(session, back);
}

#[test]
fn session_location_tagged_roundtrip() {
    let loc = SessionLocation::Voice {
        channel_id: "lounge".into(),
    };
    let json = serde_json::to_string(&loc).unwrap();
    assert!(json.contains("\"kind\":\"voice\""));
    let back: SessionLocation = serde_json::from_str(&json).unwrap();
    assert_eq!(loc, back);
    assert_eq!(back.channel_id(), "lounge");
}

#[test]
fn session_extras_roundtrip() {
    let extras = SessionExtras {
        location: Some(SessionLocation::Text {
            channel_id: "general".into(),
        }),
        activity: Some("Playing Halo".into()),
    };
    let json = serde_json::to_vec(&extras).unwrap();
    let back: SessionExtras = serde_json::from_slice(&json).unwrap();
    assert_eq!(extras, back);
}

#[test]
fn default_sharing_policy_is_deny() {
    let p = PresenceSharingPolicy::default();
    assert!(p.share_online);
    assert_eq!(p.share_location, ShareScope::None);
    assert_eq!(p.share_activity, ShareScope::None);
    assert_eq!(p.last_seen, LastSeenPrecision::Hidden);
}
