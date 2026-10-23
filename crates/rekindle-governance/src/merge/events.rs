//! `merge` events CRDT apply rules.

use crate::op::OpStamp;

use super::{EventState, GovernanceEntry, GovernanceState, PseudonymKey, ThreadState};
use crate::state::RsvpState;

pub(super) fn apply_events(
    author: &PseudonymKey,
    entry: &GovernanceEntry, stamp: OpStamp,
    state: &mut GovernanceState,
) {
    match entry {
        GovernanceEntry::ThreadCreated {
            thread_id,
            parent_channel_id,
            name,
            thread_type,
            record_key,
            invited,
            forum_tag,
            auto_archive_seconds,
        } => {
            let should_replace = state.threads.get(thread_id).is_none_or(|existing| {
                let existing_has_record = existing.record_key.is_some();
                let next_has_record = record_key.is_some();
                if existing_has_record != next_has_record {
                    return next_has_record;
                }
                if stamp != existing.created {
                    return stamp > existing.created;
                }
                author.0 > existing.creator.0
            });

            if should_replace {
                let archived = state
                    .threads
                    .get(thread_id)
                    .and_then(|existing| existing.archived)
                    .filter(|archived| *archived > stamp);
                let unarchived = state
                    .threads
                    .get(thread_id)
                    .and_then(|existing| existing.unarchived)
                    .filter(|unarchived| *unarchived > stamp);
                state.threads.insert(
                    *thread_id,
                    ThreadState {
                        parent_channel_id: *parent_channel_id,
                        name: name.clone(),
                        thread_type: thread_type.clone(),
                        record_key: record_key.clone(),
                        invited: invited.clone(),
                        forum_tag: forum_tag.clone(),
                        auto_archive_seconds: *auto_archive_seconds,
                        creator: author.clone(),
                        created: stamp,
                        archived,
                        unarchived,
                    },
                );
            }
        }

        // ── Thread archived (tombstone) ──
        GovernanceEntry::ThreadArchived { thread_id } => {
            if let Some(thread) = state.threads.get_mut(thread_id) {
                if stamp > thread.created
                    && thread
                        .archived
                        .is_none_or(|current| stamp > current)
                    && thread
                        .unarchived
                        .is_none_or(|reopened| stamp > reopened)
                {
                    thread.archived = Some(stamp);
                }
            }
        }

        // ── Thread unarchived: the later of archive and reopen wins ──
        GovernanceEntry::ThreadUnarchived { thread_id } => {
            if let Some(thread) = state.threads.get_mut(thread_id) {
                if stamp > thread.created
                    && thread
                        .unarchived
                        .is_none_or(|current| stamp > current)
                {
                    thread.unarchived = Some(stamp);
                    if thread
                        .archived
                        .is_some_and(|archived| stamp > archived)
                    {
                        thread.archived = None;
                    }
                }
            }
        }

        // ── Events: LWW per event_id ──
        GovernanceEntry::EventCreated {
            event_id,
            name,
            description,
            start_time,
            end_time,
            channel_id,
            cover_image_ref,
            creator_pseudonym,
            recurrence,
            location,
            status,
        } => {
            let existing_stamp = state.events.get(event_id).map_or(OpStamp::MIN, |e| e.stamp);
            if stamp > existing_stamp {
                state.events.insert(
                    *event_id,
                    EventState {
                        name: name.clone(),
                        description: description.clone(),
                        start_time: *start_time,
                        end_time: *end_time,
                        channel_id: *channel_id,
                        cover_image_ref: cover_image_ref.clone(),
                        creator_pseudonym: creator_pseudonym.clone(),
                        recurrence: recurrence.clone(),
                        location: location.clone(),
                        status: status.unwrap_or(rekindle_types::event::EventStatus::Scheduled),
                        stamp,
                    },
                );
            }
        }

        // ── Expressions: OR-Set by expression_id ──
        GovernanceEntry::EventArchived {
            event_id, ..
        } => {
            if let Some(event) = state.events.get(event_id) {
                if stamp > event.stamp {
                    state.events.remove(event_id);
                    state.event_rsvps.remove(event_id);
                }
            }
        }

        // ── RSVPs: LWW per (event_id, author) ──
        GovernanceEntry::EventRsvp {
            event_id,
            status,
        } => {
            let rsvps = state.event_rsvps.entry(*event_id).or_default();
            if rsvps
                .get(author)
                .is_none_or(|current| stamp > current.stamp)
            {
                rsvps.insert(
                    author.clone(),
                    RsvpState {
                        status: *status,
                        stamp,
                    },
                );
            }
        }

        // ── Onboarding: LWW ──
        _ => unreachable!("apply_events: unexpected variant"),
    }
}
