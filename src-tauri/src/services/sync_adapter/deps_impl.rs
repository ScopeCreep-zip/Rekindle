//! Phase 22 REDO — `SyncDeps` impl for `SyncAdapter`. Each method
//! body is thin — DB-bound primitives delegate via `db_helpers`,
//! the `attempt_pending_retry` body lives in the sibling
//! `attempt.rs` module so this file stays focused on dispatch.

use async_trait::async_trait;
use rekindle_sync::{PendingMessageRow, PendingRetryOutcome, SyncDeps};

use crate::services::sync_adapter::SyncAdapter;
use crate::state_helpers;

#[async_trait]
impl SyncDeps for SyncAdapter {
    fn current_owner_key(&self) -> String {
        state_helpers::owner_key_or_default(&self.state)
    }

    async fn load_pending_messages(&self, owner_key: &str) -> Vec<PendingMessageRow> {
        let owner = owner_key.to_string();
        crate::db_helpers::db_call_or_default(&self.pool, move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, recipient_key, body, retry_count \
                 FROM pending_messages WHERE owner_key = ?1 ORDER BY id",
            )?;
            let rows = stmt.query_map(rusqlite::params![owner], |row| {
                Ok(PendingMessageRow {
                    id: row.get::<_, i64>(0)?,
                    recipient_key: row.get::<_, String>(1)?,
                    body: row.get::<_, String>(2)?,
                    retry_count: row.get::<_, i64>(3)?,
                })
            })?;
            let mut results = Vec::new();
            for row in rows {
                results.push(row?);
            }
            Ok(results)
        })
        .await
    }

    async fn delete_pending_message(&self, id: i64) {
        let _ = crate::db_helpers::db_call(&self.pool, move |conn| {
            conn.execute(
                "DELETE FROM pending_messages WHERE id = ?1",
                rusqlite::params![id],
            )?;
            Ok(())
        })
        .await;
    }

    async fn increment_pending_retry(&self, id: i64) {
        let _ = crate::db_helpers::db_call(&self.pool, move |conn| {
            conn.execute(
                "UPDATE pending_messages SET retry_count = retry_count + 1 WHERE id = ?1",
                rusqlite::params![id],
            )?;
            Ok(())
        })
        .await;
    }

    async fn attempt_pending_retry(&self, row: &PendingMessageRow) -> PendingRetryOutcome {
        super::attempt::attempt_pending_retry(&self.state, row).await
    }
}
