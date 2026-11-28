use super::*;
use rekindle_types::event::RsvpStatus;
use rekindle_types::id::EventId;

const EVENT: EventId = EventId([0xE1; 16]);

fn meta() -> Stamped {
    at(
        1,
        GovernanceEntry::CommunityMeta {
            name: Some("C".into()),
            description: None,
            icon_hash: None,
            banner_hash: None,
        },
    )
}

fn created(lamport: u64) -> Stamped {
    at(
        lamport,
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
        },
    )
}

fn rsvp(status: RsvpStatus, lamport: u64) -> Stamped {
    at(
        lamport,
        GovernanceEntry::EventRsvp {
            event_id: EVENT,
            status,
        },
    )
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
    let state = merge_entries(
        &pseudo(1),
        &[
            (pseudo(1), vec![meta(), created(2)]),
            (
                pseudo(2),
                vec![rsvp(RsvpStatus::Going, 3), rsvp(RsvpStatus::Declined, 5)],
            ),
            (pseudo(3), vec![rsvp(RsvpStatus::Interested, 4)]),
        ],
    );
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
    let state = merge_entries(
        &pseudo(1),
        &[
            (
                pseudo(1),
                vec![
                    meta(),
                    created(2),
                    at(6, GovernanceEntry::EventArchived { event_id: EVENT }),
                ],
            ),
            (pseudo(2), vec![rsvp(RsvpStatus::Going, 3)]),
        ],
    );
    assert!(!state.event_rsvps.contains_key(&EVENT));
    assert!(statuses(&state).is_empty());
}

/// Compaction keeps only an event's latest create, so an RSVP can
/// merge before any surviving create; it still counts once the event
/// is live, on every peer alike.
#[test]
fn an_rsvp_merged_before_the_surviving_create_still_counts() {
    let state = merge_entries(
        &pseudo(1),
        &[
            (pseudo(1), vec![meta(), created(9)]),
            (pseudo(2), vec![rsvp(RsvpStatus::Going, 4)]),
        ],
    );
    assert_eq!(statuses(&state), vec![(pseudo(2), RsvpStatus::Going)]);
}

#[test]
fn rsvps_of_events_that_are_not_live_are_not_shown() {
    let state = merge_entries(
        &pseudo(1),
        &[
            (pseudo(1), vec![meta()]),
            (pseudo(2), vec![rsvp(RsvpStatus::Going, 4)]),
        ],
    );
    assert!(state.event_rsvps.contains_key(&EVENT));
    assert!(state.live_event_rsvps().is_empty());
}

#[test]
fn a_banned_member_cannot_rsvp() {
    let state = merge_entries(
        &pseudo(1),
        &[
            (
                pseudo(1),
                vec![
                    meta(),
                    created(2),
                    at(
                        3,
                        GovernanceEntry::BanEntry {
                            target: pseudo(2),
                            reason: None,
                            target_seq_frontier: 0,
                        },
                    ),
                ],
            ),
            (pseudo(2), vec![rsvp(RsvpStatus::Going, 4)]),
        ],
    );
    assert!(statuses(&state).is_empty());
}
