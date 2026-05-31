//! Phase 22 — pending-message retry primitives + orchestrator.
//!
//! Pure decision helpers for the offline retry queue + the
//! [`process_pending_retry_queue`] orchestrator that loops over
//! every pending row, dispatches via [`crate::SyncDeps`], and
//! tracks the delete-vs-bump-retry decision. AppState +
//! transport-specific orchestration (JSON envelope parse, DM vs
//! channel-message dispatch, Veilid DHT write) stays in src-tauri
//! behind the trait.
//!
//! Chiral split matches Phase 17/18/19/20/21.

use crate::deps::{PendingRetryOutcome, SyncDeps};

/// Architecture §32 W7 — pending messages get up to 20 retries before
/// the queue gives up (≈10 minutes at the 30s sync-loop cadence).
pub const MAX_PENDING_RETRIES: i64 = 20;

/// Sync loop tick — every 30 seconds.
pub const SYNC_LOOP_INTERVAL_MS: u64 = 30_000;

/// `true` when a pending message has exhausted its retry budget and
/// should be dropped from the queue (logged as undeliverable).
#[must_use]
pub fn should_drop_pending(retry_count: i64, max_retries: i64) -> bool {
    retry_count >= max_retries
}

/// `true` when the next retry attempt should fire. With the current
/// design every sync-loop tick attempts every still-eligible pending
/// row; this is the pure version of that decision (eligible == not
/// over-budget).
#[must_use]
pub fn is_retry_eligible(retry_count: i64, max_retries: i64) -> bool {
    !should_drop_pending(retry_count, max_retries)
}

/// Process the per-tick pending-message retry queue.
///
/// Steps:
/// 1. Look up the local user's owner key. Empty (logged out) →
///    no-op so we don't iterate other accounts' rows.
/// 2. Read every pending row for this identity in FIFO order.
/// 3. Drop rows over the retry budget (logged as undeliverable).
/// 4. Dispatch the still-eligible rows via
///    [`SyncDeps::attempt_pending_retry`]; the adapter parses the
///    body + sends via the appropriate transport.
/// 5. Apply the per-row outcome — delete on `Delivered` /
///    `Unrecognized`, bump retry on `Failed`.
pub async fn process_pending_retry_queue<D: SyncDeps>(deps: &D) {
    let owner_key = deps.current_owner_key();
    if owner_key.is_empty() {
        return;
    }
    let pending = deps.load_pending_messages(&owner_key).await;
    if pending.is_empty() {
        return;
    }
    tracing::debug!(count = pending.len(), "retrying pending messages");

    for row in pending {
        if should_drop_pending(row.retry_count, MAX_PENDING_RETRIES) {
            tracing::warn!(
                id = row.id,
                to = %row.recipient_key,
                retries = row.retry_count,
                "pending message exceeded max retries — dropping",
            );
            deps.delete_pending_message(row.id).await;
            continue;
        }
        match deps.attempt_pending_retry(&row).await {
            PendingRetryOutcome::Delivered => {
                tracing::debug!(id = row.id, "pending message delivered");
                deps.delete_pending_message(row.id).await;
            }
            PendingRetryOutcome::Failed => {
                tracing::debug!(id = row.id, "pending message retry failed");
                deps.increment_pending_retry(row.id).await;
            }
            PendingRetryOutcome::Unrecognized => {
                tracing::warn!(
                    id = row.id,
                    "unrecognized pending message format — dropping"
                );
                deps.delete_pending_message(row.id).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_retries_is_twenty() {
        assert_eq!(MAX_PENDING_RETRIES, 20);
    }

    #[test]
    fn sync_interval_is_thirty_seconds() {
        assert_eq!(SYNC_LOOP_INTERVAL_MS, 30_000);
    }

    #[test]
    fn fresh_pending_is_eligible() {
        assert!(is_retry_eligible(0, MAX_PENDING_RETRIES));
        assert!(!should_drop_pending(0, MAX_PENDING_RETRIES));
    }

    #[test]
    fn near_budget_is_still_eligible() {
        assert!(is_retry_eligible(19, MAX_PENDING_RETRIES));
        assert!(!should_drop_pending(19, MAX_PENDING_RETRIES));
    }

    #[test]
    fn at_budget_drops() {
        assert!(!is_retry_eligible(20, MAX_PENDING_RETRIES));
        assert!(should_drop_pending(20, MAX_PENDING_RETRIES));
    }

    #[test]
    fn over_budget_drops() {
        assert!(should_drop_pending(21, MAX_PENDING_RETRIES));
        assert!(should_drop_pending(100, MAX_PENDING_RETRIES));
    }

    #[test]
    fn negative_retry_count_treated_as_fresh() {
        // `pending_messages.retry_count` is `i64` because SQLite has no
        // unsigned ints; negative values shouldn't occur but should be
        // tolerated as fresh.
        assert!(is_retry_eligible(-1, MAX_PENDING_RETRIES));
    }

    // ---------- Orchestrator tests (process_pending_retry_queue) ----------

    use crate::deps::{PendingMessageRow, PendingRetryOutcome, SyncDeps};
    use async_trait::async_trait;
    use parking_lot::Mutex;
    use std::collections::VecDeque;

    #[derive(Default)]
    struct MockDeps {
        owner_key: String,
        pending: Mutex<Vec<PendingMessageRow>>,
        outcomes: Mutex<VecDeque<PendingRetryOutcome>>,
        deletes: Mutex<Vec<i64>>,
        increments: Mutex<Vec<i64>>,
        attempts: Mutex<Vec<i64>>,
    }

    #[async_trait]
    impl SyncDeps for MockDeps {
        fn current_owner_key(&self) -> String {
            self.owner_key.clone()
        }
        async fn load_pending_messages(&self, _owner_key: &str) -> Vec<PendingMessageRow> {
            std::mem::take(&mut *self.pending.lock())
        }
        async fn delete_pending_message(&self, id: i64) {
            self.deletes.lock().push(id);
        }
        async fn increment_pending_retry(&self, id: i64) {
            self.increments.lock().push(id);
        }
        async fn attempt_pending_retry(&self, row: &PendingMessageRow) -> PendingRetryOutcome {
            self.attempts.lock().push(row.id);
            self.outcomes
                .lock()
                .pop_front()
                .unwrap_or(PendingRetryOutcome::Failed)
        }
    }

    fn row(id: i64, retry: i64) -> PendingMessageRow {
        PendingMessageRow {
            id,
            recipient_key: format!("peer{id}"),
            body: format!("{{\"id\":{id}}}"),
            retry_count: retry,
        }
    }

    #[tokio::test]
    async fn empty_owner_key_is_a_no_op() {
        let deps = MockDeps::default();
        process_pending_retry_queue(&deps).await;
        assert!(deps.attempts.lock().is_empty());
        assert!(deps.deletes.lock().is_empty());
    }

    #[tokio::test]
    async fn delivered_rows_are_deleted() {
        let deps = MockDeps {
            owner_key: "me".into(),
            pending: Mutex::new(vec![row(1, 0), row(2, 5)]),
            outcomes: Mutex::new(
                [
                    PendingRetryOutcome::Delivered,
                    PendingRetryOutcome::Delivered,
                ]
                .into(),
            ),
            ..Default::default()
        };
        process_pending_retry_queue(&deps).await;
        assert_eq!(*deps.attempts.lock(), vec![1, 2]);
        assert_eq!(*deps.deletes.lock(), vec![1, 2]);
        assert!(deps.increments.lock().is_empty());
    }

    #[tokio::test]
    async fn failed_rows_bump_retry_count() {
        let deps = MockDeps {
            owner_key: "me".into(),
            pending: Mutex::new(vec![row(7, 3)]),
            outcomes: Mutex::new([PendingRetryOutcome::Failed].into()),
            ..Default::default()
        };
        process_pending_retry_queue(&deps).await;
        assert_eq!(*deps.attempts.lock(), vec![7]);
        assert!(deps.deletes.lock().is_empty());
        assert_eq!(*deps.increments.lock(), vec![7]);
    }

    #[tokio::test]
    async fn unrecognized_rows_are_dropped_without_attempt_count_bump() {
        let deps = MockDeps {
            owner_key: "me".into(),
            pending: Mutex::new(vec![row(9, 1)]),
            outcomes: Mutex::new([PendingRetryOutcome::Unrecognized].into()),
            ..Default::default()
        };
        process_pending_retry_queue(&deps).await;
        assert_eq!(*deps.attempts.lock(), vec![9]);
        assert_eq!(*deps.deletes.lock(), vec![9]);
        assert!(deps.increments.lock().is_empty());
    }

    #[tokio::test]
    async fn over_budget_rows_are_dropped_before_attempt() {
        let deps = MockDeps {
            owner_key: "me".into(),
            pending: Mutex::new(vec![row(100, MAX_PENDING_RETRIES)]),
            outcomes: Mutex::new(VecDeque::new()),
            ..Default::default()
        };
        process_pending_retry_queue(&deps).await;
        // Over budget — never attempted, immediately deleted.
        assert!(deps.attempts.lock().is_empty());
        assert_eq!(*deps.deletes.lock(), vec![100]);
    }

    #[tokio::test]
    async fn fifo_dispatch_order_matches_load_order() {
        let deps = MockDeps {
            owner_key: "me".into(),
            pending: Mutex::new(vec![row(3, 0), row(1, 0), row(2, 0)]),
            outcomes: Mutex::new(
                [
                    PendingRetryOutcome::Delivered,
                    PendingRetryOutcome::Delivered,
                    PendingRetryOutcome::Delivered,
                ]
                .into(),
            ),
            ..Default::default()
        };
        process_pending_retry_queue(&deps).await;
        // Order preserved verbatim — adapter is responsible for the
        // FIFO `ORDER BY id` SQL.
        assert_eq!(*deps.attempts.lock(), vec![3, 1, 2]);
    }
}
