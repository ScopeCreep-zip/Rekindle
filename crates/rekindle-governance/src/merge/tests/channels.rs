//! channels merge tests.

use super::*;

#[test]
fn channel_create_and_archive() {
    let creator = pseudo(1);
    let ch = channel_id(1);
    let entries = vec![
        at(
            1,
            GovernanceEntry::CommunityMeta {
                name: Some("C".into()),
                description: None,
                icon_hash: None,
                banner_hash: None,
            },
        ),
        at(
            2,
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
            },
        ),
        at(
            3,
            GovernanceEntry::ChannelCreated {
                channel_id: ch,
                name: "general".into(),
                channel_type: "text".into(),
                record_key: "VLD0:abc".into(),
                category_id: None,
                position: 0,
                parent_voice_channel_id: None,
            },
        ),
        at(4, GovernanceEntry::ChannelArchived { channel_id: ch }),
    ];
    let state = merge_entries(&[(creator, entries)]);
    assert!(
        state.channels.is_empty(),
        "archived channel should be removed"
    );
}

#[test]
fn channel_archive_with_lower_lamport_ignored() {
    let creator = pseudo(1);
    let ch = channel_id(1);
    let entries = vec![
        at(
            1,
            GovernanceEntry::CommunityMeta {
                name: Some("C".into()),
                description: None,
                icon_hash: None,
                banner_hash: None,
            },
        ),
        at(
            5,
            GovernanceEntry::ChannelCreated {
                channel_id: ch,
                name: "general".into(),
                channel_type: "text".into(),
                record_key: "VLD0:abc".into(),
                category_id: None,
                position: 0,
                parent_voice_channel_id: None,
            },
        ),
        // Archive with lower lamport than create — should be ignored
        at(3, GovernanceEntry::ChannelArchived { channel_id: ch }),
    ];
    let state = merge_entries(&[(creator, entries)]);
    assert!(
        state.channels.contains_key(&ch),
        "channel should still exist"
    );
}
