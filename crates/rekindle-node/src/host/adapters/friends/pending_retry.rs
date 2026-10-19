//! The pending-message retry on both hosts (plan E2.1, until E2.5's outbox
//! replaces it, D22): `rekindle_sync`'s budget and loop over the
//! `pending_messages` the friend operations queue, each attempt a resend
//! through `operations::dm`.

use async_trait::async_trait;
use rekindle_codec::message::envelope::MessageEnvelope;
use rekindle_sync::{PendingMessageRow, PendingRetryOutcome, SyncDeps};
use rekindle_transport::operations::dm::{self, Resend};
use tokio_util::sync::CancellationToken;

use super::HostFriends;

/// One retry pass's view of the Host; the attempts stop with `stop`.
pub(super) struct PendingRetry {
    pub(super) host: HostFriends,
    pub(super) stop: CancellationToken,
}

#[async_trait]
impl SyncDeps for PendingRetry {
    fn current_owner_key(&self) -> String {
        self.host.ctx().require_owner().unwrap_or_default()
    }

    async fn load_pending_messages(&self, owner_key: &str) -> Vec<PendingMessageRow> {
        let Ok(db) = self.host.ctx().db.current() else {
            return Vec::new();
        };
        let owner = owner_key.to_string();
        match db
            .call(move |conn| rekindle_db::repo::pending_messages::list(conn, &owner))
            .await
        {
            Ok(rows) => rows
                .into_iter()
                .map(|row| PendingMessageRow {
                    id: row.id,
                    recipient_key: row.recipient_key,
                    body: row.body,
                    retry_count: row.retry_count,
                })
                .collect(),
            Err(e) => {
                tracing::warn!(error = %e, "pending messages unreadable; retry sweep skipped");
                Vec::new()
            }
        }
    }

    async fn delete_pending_message(&self, id: i64) {
        let (Ok(owner), Ok(db)) = (
            self.host.ctx().require_owner(),
            self.host.ctx().db.current(),
        ) else {
            return;
        };
        if let Err(e) = db
            .call(move |conn| rekindle_db::repo::pending_messages::delete(conn, &owner, id))
            .await
        {
            tracing::warn!(id, error = %e, "delivered pending message not deleted");
        }
    }

    async fn increment_pending_retry(&self, id: i64) {
        let (Ok(owner), Ok(db)) = (
            self.host.ctx().require_owner(),
            self.host.ctx().db.current(),
        ) else {
            return;
        };
        if let Err(e) = db
            .call(move |conn| rekindle_db::repo::pending_messages::bump_retry(conn, &owner, id))
            .await
        {
            tracing::warn!(id, error = %e, "pending message retry not counted");
        }
    }

    async fn attempt_pending_retry(&self, row: &PendingMessageRow) -> PendingRetryOutcome {
        let Ok(envelope) = serde_json::from_str::<MessageEnvelope>(&row.body) else {
            return PendingRetryOutcome::Unrecognized;
        };
        // The session ending interrupts the attempt; the row stays for the
        // next session.
        match self
            .stop
            .run_until_cancelled(dm::resend(&self.host, &row.recipient_key, &envelope))
            .await
        {
            Some(Resend::Delivered) => PendingRetryOutcome::Delivered,
            Some(Resend::Failed) => PendingRetryOutcome::Failed,
            Some(Resend::Unsignable) => PendingRetryOutcome::Unrecognized,
            None => PendingRetryOutcome::Interrupted,
        }
    }
}
