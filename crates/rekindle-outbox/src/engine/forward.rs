//! Bring a peer's waiting rows forward (live finding L-FR2): an event that
//! makes the peer reachable again (a new route on their watched profile,
//! their accept opening the session) ends the wait at once instead of the
//! row's backoff, which counts attempts against a peer we could not reach.

use crate::deps::{OutboxDeps, OutboxError};
use crate::stream::StreamKind;
use rekindle_types::message::SendState;

use super::Outbox;

impl<D: OutboxDeps> Outbox<D> {
    /// Make every `queued` row of stream `kind:scope` due now, keeping its
    /// attempts (the backoff resumes where it was if this one fails too),
    /// and wake the engine; how many rows moved. A row that already landed
    /// (`sent`, awaiting its ack) keeps its re-ask: the peer holds it.
    ///
    /// # Errors
    /// The storage failed.
    pub async fn bring_forward(&self, kind: StreamKind, scope: &str) -> Result<usize, OutboxError> {
        let now = self.now();
        let mut moved = 0;
        for row in self.deps.stream_rows(&kind.stream(scope)).await? {
            if row.state != SendState::Queued || row.next_at <= now {
                continue;
            }
            // Written only while still `queued` (D22: never lowered).
            self.deps
                .reschedule_queued(&row.client_id, row.attempts, now)
                .await?;
            moved += 1;
        }
        if moved > 0 {
            self.wake.notify_one();
        }
        Ok(moved)
    }
}
