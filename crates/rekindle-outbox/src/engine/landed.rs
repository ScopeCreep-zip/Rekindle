//! Rows whose send another write already carried out.

use super::Outbox;
use crate::deps::{OutboxDeps, OutboxError, OutboxRow};
use crate::state::{is_unsettled, merge, StateInput};

impl<D: OutboxDeps> Outbox<D> {
    /// The rows `client_ids` whose payload a write other than their own
    /// send put where it was going: each unsettled one takes D22's local
    /// send (`sent`) and leaves its stream, so the rows behind it go on.
    /// A host calls it when it wrote the payload itself and the row's own
    /// send can never land (a re-join's `governance_op` rows aimed at the
    /// member record it replaced: the re-join wrote those ops onto its fresh
    /// record, and Veilid takes a DFLT owner subkey only from the owner key
    /// it no longer holds). Returns the rows settled.
    ///
    /// # Errors
    /// The storage failed.
    pub async fn settle_landed(
        &self,
        client_ids: &[String],
    ) -> Result<Vec<OutboxRow>, OutboxError> {
        let mut settled = Vec::new();
        for client_id in client_ids {
            let Some(row) = self.deps.row(client_id).await? else {
                continue;
            };
            if !is_unsettled(row.state) {
                continue;
            }
            let state = merge(row.state, StateInput::LocalSent);
            let moved = self.deps.settle(client_id, state).await?;
            self.publish_moved(&row, moved, None);
            settled.push(row);
        }
        if !settled.is_empty() {
            self.wake.notify_one();
        }
        Ok(settled)
    }
}
