//! Phase 20 REDO — gossip mesh adapter.
//!
//! Implements `rekindle_gossip::GossipDeps` against the live AppState
//! (the Host for member routes). The crate's `send_to_mesh`, `send_to_mesh_raw`,
//! and per-peer supervised fan-out parameterise over this trait so the
//! full pipeline (sign + dedup + lamport-bump + reliability-weighted
//! peer-select + supervised retries with route re-resolution) lives in
//! the crate, free of `veilid-core` / `tauri` / `rusqlite`.

use std::sync::Arc;

use crate::state::AppState;

pub mod deps_impl;
mod state_mutations;
mod state_reads;

/// Adapter struct: holds the `AppState` for in-memory community /
/// peer-overlay reads and mutations; a peer's route is re-read through
/// the Host's member route cache (plan E3.4 S4).
pub struct GossipAdapter {
    pub(super) state: Arc<AppState>,
}

impl GossipAdapter {
    pub fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }
}
