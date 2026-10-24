//! A landed, unacked row's re-ask backoff resets when its peer is heard
//! from again (E2.5g; Briar resets its retransmission backoff when a
//! contact's transport properties or mailbox change).

use crate::deps::{OutboxDeps, OutboxError};
use crate::engine::Outbox;
use crate::stream::StreamKind;

impl<D: OutboxDeps> Outbox<D> {
    /// The peer of the `kind` stream `scope` became reachable again (their
    /// route changed, they came online, or a frame of theirs arrived):
    /// each landed, unacked row of it is asked for again at now plus the
    /// kind's first re-ask, when that is earlier than its `next_at`. A kind
    /// with no re-ask is untouched. Returns how many moved.
    ///
    /// # Errors
    /// The storage failed.
    pub async fn reask_soon(&self, kind: StreamKind, scope: &str) -> Result<usize, OutboxError> {
        let Some(wait) = kind.first_reask() else {
            return Ok(0);
        };
        let wait = i64::try_from(wait.as_millis()).unwrap_or(i64::MAX);
        let at = self.deps.clock().now_ms_i64().saturating_add(wait);
        let moved = self.deps.reschedule_sent(&kind.stream(scope), at).await?;
        if moved > 0 {
            self.wake();
        }
        Ok(moved)
    }
}
