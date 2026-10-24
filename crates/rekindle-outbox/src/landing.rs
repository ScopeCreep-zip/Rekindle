//! The one landing writer and the reconcile pass (live A/B finding 73;
//! research `prod_research_batch9.md` Q2, Q3). A chain row lands through
//! [`Outbox::mark_landed`] alone, whichever path saw its entry land: the
//! appender's outcome (on time or late), the maintenance tick, the startup
//! pass. It is monotone (a row past `queued` is left as it is) and settles
//! the row in the same transaction when the peer's ack is already on it, as
//! the ack's own write settles a landed row; so the row settles whichever
//! input comes last, and a second path that sees the entry can never leave
//! it unsettled.

use std::collections::BTreeMap;

use rekindle_types::message::SendState;

use crate::deps::{Landed, OutboxDeps, OutboxError, OutboxRow, RowUpdate, StateChange};
use crate::engine::{Outbox, DEADLINE_REASON};
use crate::stream::StreamKind;

impl<D: OutboxDeps> Outbox<D> {
    /// The landing input of `row` (`queued`), its `attempts`th attempt,
    /// whose hint left at `sent_at`: `sent`, due at its first re-ask, or
    /// settled at the peer's ack already on it. A no-op on a row already
    /// landed or gone.
    ///
    /// # Errors
    /// The storage failed.
    pub(crate) async fn mark_landed(
        &self,
        row: &OutboxRow,
        attempts: u32,
        sent_at: i64,
    ) -> Result<(), OutboxError> {
        let next_at = self.next_after(row, SendState::Sent, attempts, sent_at);
        match self.deps.land(&row.client_id, attempts, next_at).await? {
            Landed::Unchanged => {}
            Landed::Waiting => {
                self.deps
                    .publish(StateChange::of(row, SendState::Sent, None));
            }
            // Its state was shown when the ack came (D22); the landing lets
            // the row go, and the stream head moves.
            Landed::Settled(_) => self.wake(),
        }
        Ok(())
    }

    /// Whether `stream`'s appender has an append running or waiting: the
    /// one source of "an append is in flight" (review LV22 M1), which the
    /// friend tick checks before it appends sealed entries itself, since a
    /// row can leave `queued` (failed, discarded) while its append runs.
    #[must_use]
    pub fn stream_busy(&self, stream: &str) -> bool {
        self.appends.streams().contains(stream)
    }

    /// Reconcile every `queued` chain row (the `dm` and `group_dm` streams)
    /// with our own chain, run by the friend maintenance tick and at
    /// startup: a row whose append is in flight within its deadline is
    /// skipped; one whose entry our chain records as landed is landed
    /// ([`Self::mark_landed`]); one past its `deadline` fails, shown to the
    /// user; any other with no append in flight is due now, to go to its
    /// stream's appender again (the same entry at its sealed seq). How many
    /// rows it landed.
    ///
    /// # Errors
    /// The storage failed.
    pub async fn reconcile(&self) -> Result<usize, OutboxError> {
        let gate = self.appends.gate.lock().await;
        let now = self.deps.clock().now_ms_i64();
        let mut streams: BTreeMap<String, Vec<OutboxRow>> = BTreeMap::new();
        for kind in [StreamKind::Dm, StreamKind::GroupDm] {
            for row in self.deps.prefix_rows(&kind.stream("")).await? {
                if row.state == SendState::Queued
                    && !self.appends.within_deadline(&row.client_id, now)
                {
                    streams.entry(row.stream.clone()).or_default().push(row);
                }
            }
        }
        let (mut landed, mut due) = (0, false);
        for (stream, rows) in streams {
            let seqs: Vec<i64> = rows.iter().map(|row| row.stream_seq).collect();
            let on = self.deps.on_chain(&stream, &seqs).await?;
            for row in rows {
                if on.contains(&row.stream_seq) && !row.kind.awaits_ack() {
                    // A group row settles `sent` once its entry lands (E2.6).
                    self.deps.settle(&row.client_id, SendState::Sent).await?;
                    self.deps
                        .publish(StateChange::of(&row, SendState::Sent, None));
                    landed += 1;
                } else if on.contains(&row.stream_seq) {
                    self.mark_landed(&row, row.attempts, now).await?;
                    landed += 1;
                } else if row.deadline <= now {
                    self.fail(&row, DEADLINE_REASON.to_owned()).await?;
                } else if !self.appends.in_flight(&row.client_id) && row.next_at > now {
                    let update = RowUpdate {
                        state: row.state,
                        attempts: row.attempts,
                        next_at: now,
                        deadline: row.deadline,
                        wedge_reason: None,
                    };
                    self.deps.update(&row.client_id, &update).await?;
                    due = true;
                }
            }
        }
        drop(gate);
        if due || landed > 0 {
            self.wake();
        }
        Ok(landed)
    }
}
