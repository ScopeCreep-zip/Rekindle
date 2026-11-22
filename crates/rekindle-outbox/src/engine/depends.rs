//! Rows that wait for a named row of their stream (plan HOL1, lead
//! decision 47). A peer's `friend_handshake` frames need no order among
//! themselves: the receiver gates each on what it really needs (the
//! friendship applied, `presence_key.rs`; the session, `frame.rs`; roster
//! admission for a group), so a row waits only for the row it names
//! ([`OutboxRow::depends_on`]), never for its place. As Signal-Android's
//! job manager does with a job's `dependsOn` (`JobController.onFailure`:
//! "all dependent jobs that should also be failed"), a row that fails
//! fails the rows waiting for it, and the user's Retry of it queues them
//! again with it.

use rekindle_types::message::SendState;

use super::{Enqueued, Entry, Outbox};
use crate::deps::{OutboxDeps, OutboxError, OutboxRow};
use crate::state::is_unsettled;

/// The reason a row carries that failed because the row it waited for did.
pub const DEPENDENCY_FAILED: &str = "the frame it waits for failed";

impl<D: OutboxDeps> Outbox<D> {
    /// Store `entry` with `echo`, waiting for the newest unsettled row of
    /// its stream whose client id starts with one of `waits_for` (none:
    /// it waits for nothing), taking over the dependents of the rows whose
    /// client ids start with one of `takes_over`
    /// ([`crate::NewOutboxRow::takes_over`]), and wake the engine.
    ///
    /// # Errors
    /// The storage failed; nothing was queued.
    pub async fn enqueue_waiting(
        &self,
        entry: Entry,
        waits_for: Vec<String>,
        takes_over: Vec<String>,
        echo: D::Echo,
    ) -> Result<Enqueued, OutboxError> {
        let mut row = self.new_row(entry);
        row.waits_for = waits_for;
        row.takes_over = takes_over;
        self.enqueue_row(row, echo).await
    }

    /// The rows of `row`'s stream that wait for it.
    async fn dependents(&self, row: &OutboxRow) -> Result<Vec<OutboxRow>, OutboxError> {
        let rows = self.deps.prefix_rows(&row.stream).await?;
        Ok(rows
            .into_iter()
            .filter(|r| {
                r.stream == row.stream && r.depends_on.as_deref() == Some(row.client_id.as_str())
            })
            .collect())
    }

    /// `row` failed: every unsettled row waiting for it, and every row
    /// waiting for one of those, fails with it.
    pub(crate) async fn fail_dependents(&self, row: &OutboxRow) -> Result<(), OutboxError> {
        let mut failed = vec![row.clone()];
        while let Some(next) = failed.pop() {
            for dependent in self.dependents(&next).await? {
                if is_unsettled(dependent.state) {
                    self.fail_one(&dependent, DEPENDENCY_FAILED.to_owned())
                        .await?;
                    failed.push(dependent);
                }
            }
        }
        Ok(())
    }

    /// The user retried `row`: the rows that failed with it (waiting for
    /// it, or for one of those) are queued again with it.
    pub(crate) async fn retry_dependents(&self, row: &OutboxRow) -> Result<(), OutboxError> {
        let mut retried = vec![row.clone()];
        while let Some(next) = retried.pop() {
            for dependent in self.dependents(&next).await? {
                if dependent.state == SendState::Failed {
                    self.requeue(&dependent).await?;
                    retried.push(dependent);
                }
            }
        }
        Ok(())
    }
}
