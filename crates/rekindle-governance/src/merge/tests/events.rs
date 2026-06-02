//! events merge tests.

use super::*;

#[test]
fn thread_prefers_populated_record_key() {
    let creator = pseudo(1);
    let thread_id = ThreadId([9; 16]);
    let parent_channel_id = channel_id(1);
    let entries = vec![
        GovernanceEntry::CommunityMeta {
            name: Some("C".into()),
            description: None,
            icon_hash: None,
            banner_hash: None,
            lamport: 1,
        },
        GovernanceEntry::ThreadCreated {
            thread_id,
            parent_channel_id,
            name: "ops".into(),
            thread_type: "public".into(),
            record_key: None,
            invited: Vec::new(),
            forum_tag: None,
            auto_archive_seconds: 86_400,
            lamport: 3,
        },
        GovernanceEntry::ThreadCreated {
            thread_id,
            parent_channel_id,
            name: "ops".into(),
            thread_type: "public".into(),
            record_key: Some("VLD0:thread".into()),
            invited: Vec::new(),
            forum_tag: None,
            auto_archive_seconds: 86_400,
            lamport: 4,
        },
    ];

    let state = merge(&[(creator, entries)]);
    assert_eq!(
        state
            .threads
            .get(&thread_id)
            .and_then(|thread| thread.record_key.as_deref()),
        Some("VLD0:thread")
    );
}

#[test]
fn thread_archive_sets_tombstone_without_removing_thread() {
    let creator = pseudo(1);
    let thread_id = ThreadId([7; 16]);
    let parent_channel_id = channel_id(1);
    let entries = vec![
        GovernanceEntry::CommunityMeta {
            name: Some("C".into()),
            description: None,
            icon_hash: None,
            banner_hash: None,
            lamport: 1,
        },
        GovernanceEntry::ThreadCreated {
            thread_id,
            parent_channel_id,
            name: "ops".into(),
            thread_type: "public".into(),
            record_key: Some("VLD0:thread".into()),
            invited: Vec::new(),
            forum_tag: None,
            auto_archive_seconds: 86_400,
            lamport: 3,
        },
        GovernanceEntry::ThreadArchived {
            thread_id,
            lamport: 5,
        },
    ];

    let state = merge(&[(creator, entries)]);
    let thread = state
        .threads
        .get(&thread_id)
        .expect("thread should remain materialized");
    assert_eq!(thread.archived_lamport, Some(5));
}

#[test]
fn thread_archive_with_lower_lamport_is_ignored() {
    let creator = pseudo(1);
    let thread_id = ThreadId([7; 16]);
    let parent_channel_id = channel_id(1);
    let entries = vec![
        GovernanceEntry::CommunityMeta {
            name: Some("C".into()),
            description: None,
            icon_hash: None,
            banner_hash: None,
            lamport: 1,
        },
        GovernanceEntry::ThreadCreated {
            thread_id,
            parent_channel_id,
            name: "ops".into(),
            thread_type: "public".into(),
            record_key: Some("VLD0:thread".into()),
            invited: Vec::new(),
            forum_tag: None,
            auto_archive_seconds: 86_400,
            lamport: 5,
        },
        GovernanceEntry::ThreadArchived {
            thread_id,
            lamport: 4,
        },
    ];

    let state = merge(&[(creator, entries)]);
    let thread = state
        .threads
        .get(&thread_id)
        .expect("thread should remain materialized");
    assert_eq!(thread.archived_lamport, None);
}
