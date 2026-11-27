use super::*;

#[test]
fn audit_entry_serde() {
    let entry = AuditLogEntry {
        entry_id: 42,
        actor_pseudonym: "mod_abc".into(),
        action: AuditAction::MemberKick,
        target: AuditTarget::Member("user_xyz".into()),
        changes: vec![],
        reason: Some("Spamming".into()),
        timestamp: 1_234_567_890,
        signature: vec![0u8; 64],
    };

    let json = serde_json::to_string(&entry).unwrap();
    let back: AuditLogEntry = serde_json::from_str(&json).unwrap();
    assert_eq!(back.entry_id, 42);
    assert_eq!(back.actor_pseudonym, "mod_abc");
    assert!(matches!(back.action, AuditAction::MemberKick));
}

#[test]
fn audit_action_variants_serde() {
    let actions = vec![
        AuditAction::ChannelCreate,
        AuditAction::ChannelUpdate,
        AuditAction::ChannelDelete,
        AuditAction::RoleCreate,
        AuditAction::RoleUpdate,
        AuditAction::RoleDelete,
        AuditAction::MemberRoleUpdate,
        AuditAction::MemberKick,
        AuditAction::MemberBan,
        AuditAction::MemberUnban,
        AuditAction::MemberTimeout,
        AuditAction::MemberTimeoutRemove,
        AuditAction::MessageDelete,
        AuditAction::MessagePin,
        AuditAction::CommunityUpdate,
        AuditAction::AutoModRuleCreate,
        AuditAction::AutoModRuleUpdate,
        AuditAction::AutoModActionExecuted,
    ];

    for action in &actions {
        let json = serde_json::to_string(action).unwrap();
        let back: AuditAction = serde_json::from_str(&json).unwrap();
        let json2 = serde_json::to_string(&back).unwrap();
        assert_eq!(json, json2);
    }
}

#[test]
fn audit_target_variants_serde() {
    let targets = vec![
        AuditTarget::Channel("ch_01".into()),
        AuditTarget::Role(3),
        AuditTarget::Member("user_abc".into()),
        AuditTarget::Message("msg_xyz".into()),
        AuditTarget::Community,
    ];

    for target in &targets {
        let json = serde_json::to_string(target).unwrap();
        let back: AuditTarget = serde_json::from_str(&json).unwrap();
        let json2 = serde_json::to_string(&back).unwrap();
        assert_eq!(json, json2);
    }
}

#[test]
fn audit_change_serde() {
    let change = AuditChange {
        field: "name".into(),
        old_value: Some("Old Name".into()),
        new_value: Some("New Name".into()),
    };

    let json = serde_json::to_string(&change).unwrap();
    let back: AuditChange = serde_json::from_str(&json).unwrap();
    assert_eq!(back.field, "name");
    assert_eq!(back.old_value.as_deref(), Some("Old Name"));
}

#[test]
fn audit_entry_with_changes() {
    let entry = AuditLogEntry {
        entry_id: 1,
        actor_pseudonym: "admin".into(),
        action: AuditAction::ChannelUpdate,
        target: AuditTarget::Channel("ch_01".into()),
        changes: vec![
            AuditChange {
                field: "name".into(),
                old_value: Some("general".into()),
                new_value: Some("general-chat".into()),
            },
            AuditChange {
                field: "topic".into(),
                old_value: None,
                new_value: Some("Welcome!".into()),
            },
        ],
        reason: None,
        timestamp: 1_234_567_890,
        signature: vec![0u8; 64],
    };

    let json = serde_json::to_string(&entry).unwrap();
    let back: AuditLogEntry = serde_json::from_str(&json).unwrap();
    assert_eq!(back.changes.len(), 2);
    assert_eq!(back.changes[0].field, "name");
}
