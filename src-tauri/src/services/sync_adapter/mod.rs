//! Phase 22 REDO — sync adapter.
//!
//! Implements `rekindle_sync::SyncDeps` against the live AppState +
//! Db. The crate's `process_pending_retry_queue` orchestrator
//! parameterises over this trait so the loop logic + retry-budget
//! decision stay in the crate, while AppState reads + Veilid
//! transport + DB writes stay in src-tauri (Invariant 7).

use std::sync::Arc;

use crate::state::AppState;
use rekindle_db::Db;

pub mod attempt;
pub mod deps_impl;

/// Adapter struct — same shape as Phase 17/18/19/20/21 adapters.
pub struct SyncAdapter {
    pub(super) state: Arc<AppState>,
    pub(super) pool: Db,
    /// The tick's stop token: an attempt's record-pool work is left when
    /// the session ends (the pool runs each Veilid call on its own scope,
    /// so none is dropped mid-flight).
    pub(super) stop: tokio_util::sync::CancellationToken,
}

impl SyncAdapter {
    pub fn new(state: Arc<AppState>, pool: Db, stop: tokio_util::sync::CancellationToken) -> Self {
        Self { state, pool, stop }
    }
}

/// Build a one-shot adapter — the sync-loop caller already owns the
/// pool (it's a setup-time parameter), so this builder is a thin
/// `Arc` clone. Future callers that don't have the pool can look
/// it up via `app_handle.try_state::<Db>()` before invoking
/// this.
pub fn build_adapter(
    state: &Arc<AppState>,
    pool: Db,
    stop: &tokio_util::sync::CancellationToken,
) -> SyncAdapter {
    SyncAdapter::new(Arc::clone(state), pool, stop.clone())
}

/// Run one pending-message retry tick. The crate's
/// `process_pending_retry_queue` orchestrator owns the loop +
/// retry-budget decision; this facade builds the adapter and
/// delegates. Used by `sync_service::retry_pending_messages` (the
/// periodic tick caller).
pub async fn run_pending_retry_tick(
    state: &Arc<AppState>,
    pool: &Db,
    stop: &tokio_util::sync::CancellationToken,
) {
    let adapter = build_adapter(state, pool.clone(), stop);
    rekindle_sync::process_pending_retry_queue(&adapter, stop).await;
}
