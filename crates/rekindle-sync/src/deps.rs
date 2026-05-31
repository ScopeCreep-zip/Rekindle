//! Phase 22 REDO — `SyncDeps` composite trait + DTOs.
//!
//! Bag of operations the sync orchestrators need from their host.
//! Implemented in src-tauri by `SyncAdapter` against the live
//! `AppState` + `DbPool` + message-service helpers. Chiral split
//! pattern matches Phase 17/18/19/20/21 REDO.

use async_trait::async_trait;

/// One row from the `pending_messages` SQLite table. The adapter
/// reads + returns these; the crate orchestrator dispatches by
/// trying to decode `body` as a DM envelope first, then as a
/// pending channel message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingMessageRow {
    pub id: i64,
    pub recipient_key: String,
    /// JSON-encoded body — the orchestrator decodes as either
    /// `MessageEnvelope` (DM) or `PendingChannelMessage` (channel).
    pub body: String,
    pub retry_count: i64,
}

/// Outcome of one retry attempt the adapter reports back.
///
/// `Delivered` → adapter should delete the row.
/// `Failed` → adapter should increment `retry_count`.
/// `Unrecognized` → adapter should delete (body doesn't decode).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingRetryOutcome {
    Delivered,
    Failed,
    Unrecognized,
}

/// Composite trait covering every external touchpoint the sync
/// orchestrators need. Implemented by `SyncAdapter` in src-tauri.
#[async_trait]
pub trait SyncDeps: Send + Sync + 'static {
    /// Local user's owner key (Ed25519 public hex) — pending-message
    /// rows are scoped per identity to prevent cross-account leakage.
    /// Returns empty string when no identity is loaded.
    fn current_owner_key(&self) -> String;

    /// Load every pending-message row for the current identity in
    /// ID-ascending order (FIFO retry semantics).
    async fn load_pending_messages(&self, owner_key: &str) -> Vec<PendingMessageRow>;

    /// Drop a pending-message row (delivered, exceeded retry budget,
    /// or body unrecognised).
    async fn delete_pending_message(&self, id: i64);

    /// Bump `retry_count` on a still-eligible row that failed its
    /// most recent attempt.
    async fn increment_pending_retry(&self, id: i64);

    /// Attempt to deliver one pending row. The adapter parses the
    /// body, dispatches via the appropriate transport (DM via
    /// message_service::send_to_peer_call, channel via DHTManager
    /// write), and reports the outcome back. Keeps the JSON parse +
    /// transport-specific orchestration in src-tauri (Invariant 7)
    /// while the crate owns the retry loop + eligibility decision.
    async fn attempt_pending_retry(&self, row: &PendingMessageRow) -> PendingRetryOutcome;
}
