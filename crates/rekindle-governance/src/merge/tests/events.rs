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

mod rsvps {
    use super::*;
    use rekindle_types::event::RsvpStatus;
    use rekindle_types::id::EventId;

    const EVENT: EventId = EventId([0xE1; 16]);

    fn meta() -> GovernanceEntry {
        GovernanceEntry::CommunityMeta {
            name: Some("C".into()),
            description: None,
            icon_hash: None,
            banner_hash: None,
            lamport: 1,
        }
    }

    fn created(lamport: u64) -> GovernanceEntry {
        GovernanceEntry::EventCreated {
            event_id: EVENT,
            name: "raid night".into(),
            description: None,
            start_time: 1_800_000_000,
            end_time: None,
            channel_id: None,
            cover_image_ref: None,
            creator_pseudonym: None,
            recurrence: None,
            location: None,
            status: None,
            lamport,
        }
    }

    fn rsvp(status: RsvpStatus, lamport: u64) -> GovernanceEntry {
        GovernanceEntry::EventRsvp {
            event_id: EVENT,
            status,
            lamport,
        }
    }

    fn statuses(state: &GovernanceState) -> Vec<(PseudonymKey, RsvpStatus)> {
        state
            .live_event_rsvps()
            .into_iter()
            .find(|(id, _)| *id == EVENT)
            .map(|(_, list)| list)
            .unwrap_or_default()
    }

    #[test]
    fn each_member_keeps_their_latest_answer() {
        let state = merge(&[
            (pseudo(1), vec![meta(), created(2)]),
            (
                pseudo(2),
                vec![rsvp(RsvpStatus::Going, 3), rsvp(RsvpStatus::Declined, 5)],
            ),
            (pseudo(3), vec![rsvp(RsvpStatus::Interested, 4)]),
        ]);
        assert_eq!(
            statuses(&state),
            vec![
                (pseudo(2), RsvpStatus::Declined),
                (pseudo(3), RsvpStatus::Interested),
            ]
        );
    }

    #[test]
    fn archiving_the_event_clears_its_rsvps() {
        let state = merge(&[
            (
                pseudo(1),
                vec![
                    meta(),
                    created(2),
                    GovernanceEntry::EventArchived {
                        event_id: EVENT,
                        lamport: 6,
                    },
                ],
            ),
            (pseudo(2), vec![rsvp(RsvpStatus::Going, 3)]),
        ]);
        assert!(!state.event_rsvps.contains_key(&EVENT));
        assert!(statuses(&state).is_empty());
    }

    /// Compaction keeps only an event's latest create, so an RSVP can
    /// merge before any surviving create; it still counts once the event
    /// is live, on every peer alike.
    #[test]
    fn an_rsvp_merged_before_the_surviving_create_still_counts() {
        let state = merge(&[
            (pseudo(1), vec![meta(), created(9)]),
            (pseudo(2), vec![rsvp(RsvpStatus::Going, 4)]),
        ]);
        assert_eq!(statuses(&state), vec![(pseudo(2), RsvpStatus::Going)]);
    }

    #[test]
    fn rsvps_of_events_that_are_not_live_are_not_shown() {
        let state = merge(&[
            (pseudo(1), vec![meta()]),
            (pseudo(2), vec![rsvp(RsvpStatus::Going, 4)]),
        ]);
        assert!(state.event_rsvps.contains_key(&EVENT));
        assert!(state.live_event_rsvps().is_empty());
    }

    #[test]
    fn a_banned_member_cannot_rsvp() {
        let state = merge(&[
            (
                pseudo(1),
                vec![
                    meta(),
                    created(2),
                    GovernanceEntry::BanEntry {
                        target: pseudo(2),
                        reason: None,
                        lamport: 3,
                    },
                ],
            ),
            (pseudo(2), vec![rsvp(RsvpStatus::Going, 4)]),
        ]);
        assert!(statuses(&state).is_empty());
    }
}
