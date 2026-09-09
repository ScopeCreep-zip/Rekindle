//! `merge` events CRDT apply rules.

use super::{EventState, GovernanceEntry, GovernanceState, PseudonymKey, ThreadState};

pub(super) fn apply_events(
    author: &PseudonymKey,
    entry: &GovernanceEntry,
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
            lamport,
        } => {
            let should_replace = state.threads.get(thread_id).is_none_or(|existing| {
                let existing_has_record = existing.record_key.is_some();
                let next_has_record = record_key.is_some();
                if existing_has_record != next_has_record {
                    return next_has_record;
                }
                if *lamport != existing.created_lamport {
                    return *lamport > existing.created_lamport;
                }
                author.0 > existing.creator.0
            });

            if should_replace {
                let archived_lamport = state
                    .threads
                    .get(thread_id)
                    .and_then(|existing| existing.archived_lamport)
                    .filter(|archived| *archived > *lamport);
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
                        created_lamport: *lamport,
                        archived_lamport,
                    },
                );
            }
        }

        // ── Thread archived (tombstone) ──
        GovernanceEntry::ThreadArchived { thread_id, lamport } => {
            if let Some(thread) = state.threads.get_mut(thread_id) {
                if *lamport > thread.created_lamport
                    && thread
                        .archived_lamport
                        .is_none_or(|current| *lamport > current)
                {
                    thread.archived_lamport = Some(*lamport);
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
            lamport,
        } => {
            let existing_lamport = state.events.get(event_id).map_or(0, |e| e.lamport);
            if *lamport > existing_lamport {
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
                        lamport: *lamport,
                    },
                );
            }
        }

        // ── Expressions: OR-Set by expression_id ──
        GovernanceEntry::EventArchived {
            event_id, lamport, ..
        } => {
            if let Some(event) = state.events.get(event_id) {
                if *lamport > event.lamport {
                    state.events.remove(event_id);
                }
            }
        }

        // ── Onboarding: LWW ──
        _ => unreachable!("apply_events: unexpected variant"),
    }
}
