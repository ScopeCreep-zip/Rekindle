//! Phase 20 REDO — gossip mesh adapter.
//!
//! Implements `rekindle_gossip::GossipDeps` against the live AppState
//! + AppHandle + DbPool. The crate's `send_to_mesh`, `send_to_mesh_raw`,
//! and per-peer supervised fan-out parameterise over this trait so the
//! full pipeline (sign + dedup + lamport-bump + reliability-weighted
//! peer-select + supervised retries with route re-resolution) lives in
//! the crate, free of `veilid-core` / `tauri` / `rusqlite`.

use std::sync::Arc;

use crate::db::DbPool;
use crate::state::AppState;

pub mod deps_impl;

/// Adapter struct — holds the `AppState` (for in-memory community /
/// peer-overlay reads and mutations) and a `DbPool` clone (for the
/// `record_delivery` SQLite write + the `community_members` subkey
/// lookup performed during DHT route re-resolution). The `AppHandle`
/// is consumed at construction time in `deps_impl::build_adapter` to
/// extract the pool; the adapter never needs it again.
pub struct GossipAdapter {
    pub(super) state: Arc<AppState>,
    pub(super) pool: DbPool,
}

impl GossipAdapter {
    pub fn new(state: Arc<AppState>, pool: DbPool) -> Self {
        Self { state, pool }
    }
}
