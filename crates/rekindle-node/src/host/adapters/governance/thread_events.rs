//! Thread archive verdicts from the merged governance (plan E7.13 walk
//! 18 (a)), on both modes. An archive or a reopening is a governance entry
//! (`ThreadArchived` / `ThreadUnarchived`, `rekindle-channel`
//! `threads/lifecycle.rs`), so, like membership (`member_events.rs`), it is
//! read as the difference between the state last projected and the state
//! just merged, and published once as `ThreadArchiveChanged`. That covers
//! our own archive (the local merge of our own entry: the mesh never
//! echoes a frame to its sender) and every other member's (the merge of
//! the op they fetched), as Discord shows an archive to the archiver and
//! every member at once. The stored thread row follows first, queued on
//! the database's one connection, so a re-read after the event agrees.
//!
//! A thread new to the merged state is published once as `ThreadCreated`
//! the same way (plan E3.2 S8-E): the signed `ThreadCreated` entry is the
//! only source, never an unsigned gossip frame. It is read from the state
//! as projected, so a thread held from a timed-out creator
//! (`hold_timed_out_threads`) is never announced. Its row is not written
//! here: `list_threads` reads every thread from the merged state and the
//! stored row adds only what the creator alone knows (the starter
//! message), which the creator's own `create_thread` stores.

use rekindle_governance::state::GovernanceState;
use rekindle_types::subscription_events::{SocialEvent, SubscriptionEvent};
use rekindle_types::thread::ThreadInfo;

use super::HostGovernance;

/// Each thread whose manual archive state differs between `prev` and
/// `next`, as `(thread id hex, archived)`, in thread id order. A thread in
/// only one of the two (created, or held back) is no change of archive.
pub(super) fn thread_archive_transitions(
    prev: &GovernanceState,
    next: &GovernanceState,
) -> Vec<(String, bool)> {
    let mut changes: Vec<(String, bool)> = next
        .threads
        .iter()
        .filter_map(|(id, thread)| {
            let before = prev.threads.get(id)?.archived.is_some();
            let now = thread.archived.is_some();
            (before != now).then(|| (hex::encode(id.0), now))
        })
        .collect();
    changes.sort();
    changes
}

/// One `ThreadCreated` of `community_id` per thread in `next` and not in
/// `prev`, in thread id order, dated `now_secs` (the entry carries no time,
/// so the moment it reaches us is its time here, as `ingest.rs` judges
/// it). The governance entry names no starter message, so it is empty, as
/// `list_threads` reads a thread with no stored row.
pub(super) fn thread_creations(
    community_id: &str,
    prev: &GovernanceState,
    next: &GovernanceState,
    now_secs: u64,
) -> Vec<SocialEvent> {
    let mut created: Vec<_> = next
        .threads
        .iter()
        .filter(|(id, _)| !prev.threads.contains_key(*id))
        .collect();
    created.sort_by_key(|(id, _)| id.0);
    created
        .into_iter()
        .map(|(id, thread)| SocialEvent::ThreadCreated {
            community: community_id.to_string(),
            thread: Box::new(ThreadInfo {
                id: hex::encode(id.0),
                channel_id: hex::encode(thread.parent_channel_id.0),
                name: thread.name.clone(),
                starter_message_id: String::new(),
                creator_pseudonym: hex::encode(thread.creator.0),
                forum_tag: thread.forum_tag.clone(),
                created_at: now_secs,
                archived: thread.archived.is_some(),
                auto_archive_seconds: u32::try_from(thread.auto_archive_seconds)
                    .unwrap_or(u32::MAX),
                last_message_at: 0,
                message_count: 0,
            }),
        })
        .collect()
}

impl HostGovernance {
    /// Publish each thread `projected` holds and `prev` did not (from
    /// [`thread_creations`]). Nothing on the first merge of a session:
    /// that is a load, which the thread list reads itself.
    pub(super) fn publish_thread_creations(
        &self,
        community_id: &str,
        prev: Option<&GovernanceState>,
        now_secs: u64,
    ) {
        let Some(prev) = prev else { return };
        let Some(projected) = self.ctx.communities.governance_state(community_id) else {
            return;
        };
        for event in thread_creations(community_id, prev, &projected, now_secs) {
            self.ctx.publish_event(SubscriptionEvent::Social(event));
        }
    }

    /// Store and publish `changes` (from [`thread_archive_transitions`])
    /// of `community_id`.
    pub(super) fn publish_thread_archive_changes(
        &self,
        community_id: &str,
        changes: Vec<(String, bool)>,
    ) {
        if changes.is_empty() {
            return;
        }
        if let Some((owner, db)) = self.owner_and_db() {
            let (community, rows) = (community_id.to_string(), changes.clone());
            db.fire("thread archive", move |conn| {
                for (thread_id, archived) in &rows {
                    rekindle_db::repo::threads::set_archived(
                        conn, &owner, &community, thread_id, *archived,
                    )?;
                }
                Ok(())
            });
        }
        for (thread_id, archived) in changes {
            self.ctx.publish_event(SubscriptionEvent::Social(
                SocialEvent::ThreadArchiveChanged {
                    community: community_id.to_string(),
                    thread_id,
                    archived,
                },
            ));
        }
    }
}

#[cfg(test)]
#[path = "thread_events_tests.rs"]
mod tests;
