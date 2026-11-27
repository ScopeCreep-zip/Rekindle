use super::*;

#[test]
fn invite_secrets_serde_roundtrip() {
    let secrets = InviteSecrets {
        anchor: "VLD0:anchor".into(),
        inviter_member_record: "VLD0:member".into(),
        inviter_pseudonym: "ab".repeat(32),
        community_name: "Test Community".into(),
        registry_key: "VLD0:reg456".into(),
        slot_seed: "ab".repeat(32),
        join_inbox: None,
    };
    let json = serde_json::to_string(&secrets).unwrap();
    let back: InviteSecrets = serde_json::from_str(&json).unwrap();
    assert_eq!(back.anchor, "VLD0:anchor");
    assert_eq!(back.inviter_member_record, "VLD0:member");
    assert_eq!(back.inviter_pseudonym, "ab".repeat(32));
    assert!(back.join_inbox.is_none());
    assert!(!json.contains("joinInbox"));

    let secrets = InviteSecrets {
        join_inbox: Some(JoinInboxCapability {
            join_inbox: "VLD0:inbox".into(),
            join_inbox_writer: Zeroizing::new("writer is secret".into()),
            join_inbox_slots: 3,
        }),
        ..secrets
    };
    assert!(!format!("{secrets:?}").contains("writer is secret"));
    let json = serde_json::to_value(&secrets).unwrap();
    assert_eq!(json["joinInbox"], "VLD0:inbox");
    assert_eq!(json["joinInboxSlots"], 3);
    let back: InviteSecrets = serde_json::from_value(json).unwrap();
    assert_eq!(
        &*back.join_inbox.unwrap().join_inbox_writer,
        "writer is secret"
    );
}
