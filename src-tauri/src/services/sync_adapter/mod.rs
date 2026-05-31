//! Phase 22 REDO — sync adapter.
//!
//! Implements `rekindle_sync::SyncDeps` against the live AppState +
//! DbPool. The crate's `process_pending_retry_queue` orchestrator
//! parameterises over this trait so the loop logic + retry-budget
//! decision stay in the crate, while AppState reads + Veilid
//! transport + DB writes stay in src-tauri (Invariant 7).

use std::sync::Arc;

use crate::db::DbPool;
use crate::state::AppState;

pub mod attempt;
pub mod deps_impl;

/// Adapter struct — same shape as Phase 17/18/19/20/21 adapters.
pub struct SyncAdapter {
    pub(super) state: Arc<AppState>,
    pub(super) pool: DbPool,
}

impl SyncAdapter {
    pub fn new(state: Arc<AppState>, pool: DbPool) -> Self {
        Self { state, pool }
    }
}

/// Build a one-shot adapter — the sync-loop caller already owns the
/// pool (it's a setup-time parameter), so this builder is a thin
/// `Arc` clone. Future callers that don't have the pool can look
/// it up via `app_handle.try_state::<DbPool>()` before invoking
/// this.
pub fn build_adapter(state: &Arc<AppState>, pool: DbPool) -> SyncAdapter {
    SyncAdapter::new(Arc::clone(state), pool)
}

/// Run one pending-message retry tick. The crate's
/// `process_pending_retry_queue` orchestrator owns the loop +
/// retry-budget decision; this facade builds the adapter and
/// delegates. Used by `sync_service::retry_pending_messages` (the
/// periodic tick caller).
pub async fn run_pending_retry_tick(state: &Arc<AppState>, pool: &DbPool) {
    let adapter = build_adapter(state, pool.clone());
    rekindle_sync::process_pending_retry_queue(&adapter).await;
}
