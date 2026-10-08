//! expression merge tests.

use super::*;

#[test]
fn expression_or_set_remove_with_higher_lamport_wins() {
    let creator = pseudo(1);
    let expression_id = [7_u8; 16];
    let entries = vec![
        GovernanceEntry::CommunityMeta {
            name: Some("C".into()),
            description: None,
            icon_hash: None,
            banner_hash: None,
            lamport: 1,
        },
        GovernanceEntry::RoleDefinition {
            role_id: role_id(0),
            name: "everyone".into(),
            permissions: rekindle_types::permissions::ALL,
            position: 0,
            color: 0,
            hoist: false,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
            lamport: 2,
        },
        GovernanceEntry::ExpressionAdded {
            expression_id,
            name: "wave".into(),
            kind: "emoji".into(),
            content_hash: "hash-a".into(),
            attachment: None,
            animated: false,
            tags: vec![],
            sound_meta: None,
            creator_pseudonym: None,
            created_at: None,
            available_to_peers: Some(true),
            lamport: 3,
        },
        GovernanceEntry::ExpressionRemoved {
            expression_id,
            lamport: 4,
        },
    ];

    let state = merge(&[(creator, entries)]);
    assert!(!state.expressions.contains_key(&expression_id));
}

#[test]
fn expression_or_set_add_after_remove_reappears() {
    let creator = pseudo(1);
    let expression_id = [8_u8; 16];
    let entries = vec![
        GovernanceEntry::CommunityMeta {
            name: Some("C".into()),
            description: None,
            icon_hash: None,
            banner_hash: None,
            lamport: 1,
        },
        GovernanceEntry::RoleDefinition {
            role_id: role_id(0),
            name: "everyone".into(),
            permissions: rekindle_types::permissions::ALL,
            position: 0,
            color: 0,
            hoist: false,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
            lamport: 2,
        },
        GovernanceEntry::ExpressionRemoved {
            expression_id,
            lamport: 3,
        },
        GovernanceEntry::ExpressionAdded {
            expression_id,
            name: "spark".into(),
            kind: "emoji".into(),
            content_hash: "hash-b".into(),
            attachment: None,
            animated: true,
            tags: vec!["fun".into()],
            sound_meta: None,
            creator_pseudonym: None,
            created_at: None,
            available_to_peers: Some(true),
            lamport: 4,
        },
    ];

    let state = merge(&[(creator, entries)]);
    let expression = state
        .expressions
        .get(&expression_id)
        .expect("expression should exist");
    assert_eq!(expression.name, "spark");
    assert!(expression.animated);
}

#[test]
fn attachment_pin_lww_pin_then_unpin_then_repin() {
    // Sequential lamports — final state matches the highest-lamport entry.
    let creator = pseudo(1);
    let attachment_id = [9u8; 16];
    let entries = vec![
        GovernanceEntry::AttachmentPinned {
            attachment_id,
            pinned: true,
            lamport: 1,
        },
        GovernanceEntry::AttachmentPinned {
            attachment_id,
            pinned: false,
            lamport: 2,
        },
        GovernanceEntry::AttachmentPinned {
            attachment_id,
            pinned: true,
            lamport: 3,
        },
    ];
    let state = merge(&[(creator, entries)]);
    assert!(state.pinned_attachments.contains(&attachment_id));
    assert_eq!(state.attachment_pin_lamports.get(&attachment_id), Some(&3));
}

#[test]
fn attachment_pin_out_of_order_arrival_converges() {
    // Same entries, reverse order — LWW must produce the same final state.
    let creator = pseudo(2);
    let attachment_id = [7u8; 16];
    let entries = vec![
        GovernanceEntry::AttachmentPinned {
            attachment_id,
            pinned: true,
            lamport: 5,
        },
        GovernanceEntry::AttachmentPinned {
            attachment_id,
            pinned: false,
            lamport: 3,
        },
        GovernanceEntry::AttachmentPinned {
            attachment_id,
            pinned: true,
            lamport: 1,
        },
    ];
    let state = merge(&[(creator, entries)]);
    assert!(state.pinned_attachments.contains(&attachment_id));
    assert_eq!(state.attachment_pin_lamports.get(&attachment_id), Some(&5));
}

#[test]
fn attachment_unpin_at_highest_lamport_clears() {
    let creator = pseudo(3);
    let attachment_id = [4u8; 16];
    let entries = vec![
        GovernanceEntry::AttachmentPinned {
            attachment_id,
            pinned: true,
            lamport: 10,
        },
        GovernanceEntry::AttachmentPinned {
            attachment_id,
            pinned: false,
            lamport: 11,
        },
    ];
    let state = merge(&[(creator, entries)]);
    assert!(!state.pinned_attachments.contains(&attachment_id));
    assert_eq!(state.attachment_pin_lamports.get(&attachment_id), Some(&11));
}

#[test]
fn attachment_pin_independent_per_attachment() {
    let creator = pseudo(4);
    let id_a = [1u8; 16];
    let id_b = [2u8; 16];
    let entries = vec![
        GovernanceEntry::AttachmentPinned {
            attachment_id: id_a,
            pinned: true,
            lamport: 1,
        },
        GovernanceEntry::AttachmentPinned {
            attachment_id: id_b,
            pinned: false,
            lamport: 1,
        },
        GovernanceEntry::AttachmentPinned {
            attachment_id: id_b,
            pinned: true,
            lamport: 2,
        },
    ];
    let state = merge(&[(creator, entries)]);
    assert!(state.pinned_attachments.contains(&id_a));
    assert!(state.pinned_attachments.contains(&id_b));
}
