//! events merge tests.

use super::*;

#[test]
fn thread_archive_sets_tombstone_without_removing_thread() {
    let creator = pseudo(1);
    let thread_id = ThreadId([7; 16]);
    let parent_channel_id = channel_id(1);
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
            3,
            GovernanceEntry::ThreadCreated {
                thread_id,
                parent_channel_id,
                name: "ops".into(),
                thread_type: "public".into(),
                invited: Vec::new(),
                forum_tag: None,
                auto_archive_seconds: 86_400,
            },
        ),
        at(5, GovernanceEntry::ThreadArchived { thread_id }),
    ];

    let state = merge_entries(&creator, &[(creator.clone(), entries)]);
    let thread = state
        .threads
        .get(&thread_id)
        .expect("thread should remain materialized");
    assert_eq!(thread.archived.map(|s| s.lamport), Some(5));
}

#[test]
fn thread_archive_with_lower_lamport_is_ignored() {
    let creator = pseudo(1);
    let thread_id = ThreadId([7; 16]);
    let parent_channel_id = channel_id(1);
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
            GovernanceEntry::ThreadCreated {
                thread_id,
                parent_channel_id,
                name: "ops".into(),
                thread_type: "public".into(),
                invited: Vec::new(),
                forum_tag: None,
                auto_archive_seconds: 86_400,
            },
        ),
        at(4, GovernanceEntry::ThreadArchived { thread_id }),
    ];

    let state = merge_entries(&creator, &[(creator.clone(), entries)]);
    let thread = state
        .threads
        .get(&thread_id)
        .expect("thread should remain materialized");
    assert_eq!(thread.archived, None);
}

#[path = "events_rsvps.rs"]
mod rsvps;
