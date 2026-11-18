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

use rekindle_governance::state::GovernanceState;
use rekindle_types::subscription_events::{SocialEvent, SubscriptionEvent};

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

impl HostGovernance {
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
