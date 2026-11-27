//! Batch insertion of outbox rows with the local state they publish.

use rekindle_db::repo::outbox as repo;
use rekindle_db::rusqlite::{self, Connection};
use rekindle_outbox::{NewOutboxRow, OutboxError, StateChange};
use rekindle_types::message::SendState;

use super::{storage, HostOutbox};

impl HostOutbox {
    /// Store `rows` in one transaction (all or none), publish each as
    /// `queued`; the caller wakes the engine (review RV4 M3).
    ///
    /// # Errors
    /// The storage failed; nothing was stored.
    pub(super) async fn insert_all(
        &self,
        rows: Vec<(NewOutboxRow, Option<String>)>,
        additional_writes: impl FnOnce(&Connection, &str) -> rusqlite::Result<()> + Send + 'static,
    ) -> Result<Vec<i64>, OutboxError> {
        let (db, owner) = self.db()?;
        let changes = self.changes.clone();
        db.call(move |conn| {
            let new: Vec<_> = rows
                .iter()
                .map(|(row, dependency)| repo::NewRow {
                    owner_key: &owner,
                    stream: &row.stream,
                    client_id: &row.client_id,
                    kind: row.kind.as_str(),
                    payload: &row.payload,
                    next_at: row.next_at,
                    deadline: row.deadline,
                    depends_on: dependency.as_deref(),
                })
                .collect();
            let seqs = repo::enqueue_all(conn, &new, |tx| additional_writes(tx, &owner))?;
            for ((row, _), &stream_seq) in rows.into_iter().zip(&seqs) {
                let _ = changes.send(StateChange {
                    stream: row.stream,
                    stream_seq,
                    client_id: row.client_id,
                    kind: row.kind,
                    state: SendState::Queued,
                    wedge_reason: None,
                    retry: None,
                });
            }
            Ok(seqs)
        })
        .await
        .map_err(|e| storage(&e))
    }
}
