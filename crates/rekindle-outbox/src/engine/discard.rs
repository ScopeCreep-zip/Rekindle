//! Rows that leave their stream `failed` without being sent: the user's
//! Discard, a scope's discard, and the echo of rows the host already
//! failed in its own transaction.

use rekindle_types::message::SendState;

use super::Outbox;
use crate::deps::{OutboxDeps, OutboxError, OutboxRow, StateChange};
use crate::state::is_unsettled;
use crate::stream::StreamKind;

impl<D: OutboxDeps> Outbox<D> {
    /// The user's Discard: the row settles `failed` and leaves its stream,
    /// so the rows behind it go on.
    ///
    /// # Errors
    /// The storage failed.
    pub async fn discard(&self, client_id: &str) -> Result<bool, OutboxError> {
        let Some(row) = self.deps.row(client_id).await? else {
            return Ok(false);
        };
        if !is_unsettled(row.state) && row.state != SendState::Failed {
            return Ok(false);
        }
        self.deps.settle(client_id, SendState::Failed).await?;
        self.deps.publish(StateChange::of(
            &row,
            SendState::Failed,
            row.wedge_reason.clone(),
        ));
        self.wake.notify_one();
        Ok(true)
    }

    /// Discard every unsettled row of the `kind` streams whose scope starts
    /// with `scope_prefix` (a call session ended: its `media_key` rows go,
    /// whoever they were for); each settles `failed` with `reason`.
    /// Returns how many were discarded.
    ///
    /// # Errors
    /// The storage failed.
    pub async fn discard_streams(
        &self,
        kind: StreamKind,
        scope_prefix: &str,
        reason: &str,
    ) -> Result<usize, OutboxError> {
        let rows = self.deps.prefix_rows(&kind.stream(scope_prefix)).await?;
        for row in &rows {
            self.deps.settle(&row.client_id, SendState::Failed).await?;
            self.deps.publish(StateChange::of(
                row,
                SendState::Failed,
                Some(reason.to_owned()),
            ));
        }
        Ok(rows.len())
    }

    /// Rows its host already settled `failed` and deleted in a transaction
    /// of its own (a friend change that ended their friendship, review RV5
    /// item 3): publish each one's state with `reason` and wake the pump,
    /// so subscribers learn it and the streams behind them go on. Nothing
    /// is stored.
    pub fn echo_failed(&self, rows: &[OutboxRow], reason: &str) {
        for row in rows {
            self.deps.publish(StateChange::of(
                row,
                SendState::Failed,
                Some(reason.to_owned()),
            ));
        }
        if !rows.is_empty() {
            self.wake.notify_one();
        }
    }
}
