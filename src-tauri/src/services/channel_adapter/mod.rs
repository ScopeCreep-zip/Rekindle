//! Phase 19.h-REDO — channel messaging adapter.
//!
//! Implements `rekindle_channel::ChannelMessagingDeps` against the
//! live AppState + AppHandle + DbPool. The crate's
//! `send_channel_message`, `forward_channel_message`,
//! `process_retry_write`, plus reactions/mentions/threads/expressions
//! orchestrators parameterise over this trait — Phase 14.r module-dir
//! pattern, same shape as Phase 18 governance_adapter.
//!
//! Schwarzschild boundary: this is the only place in src-tauri that
//! both holds `Arc<AppState>` AND constructs `veilid_core::*` types
//! for the crate's behalf. Phase 23.D.4 split each method body class
//! into a focused submodule so `deps_impl.rs` stays under the
//! 500-LoC cap (Invariant 1):
//!
//! * `state_reads` — channel/thread/member lookups + permission compute.
//! * `dht`         — DHT write/read of channel messages, forwards,
//!   reactions, and lazy-thread record creation.
//! * `persist`     — SQLite persists for messages/threads/sequences,
//!   slowmode state, retry-queue enqueue.
//! * `events`      — `ChannelEvent` → tracing + `ChatEvent`/`CommunityEvent`
//!   local echo + delivery state emits.

use std::sync::Arc;

use tauri::AppHandle;

use crate::db::DbPool;
use crate::state::AppState;

pub mod deps_impl;
mod dht;
mod events;
mod misc;
mod persist;
mod state_mutations;
mod state_reads;

/// Adapter struct — holds the three things every trait method needs:
/// the shared `AppState`, the Tauri `AppHandle` (for event emit +
/// DbPool lookup), and the `DbPool` clone (for SQLite reads/writes).
pub struct ChannelAdapter {
    pub(super) state: Arc<AppState>,
    pub(super) app_handle: AppHandle,
    pub(super) pool: DbPool,
}

impl ChannelAdapter {
    pub fn new(state: Arc<AppState>, app_handle: AppHandle, pool: DbPool) -> Self {
        Self {
            state,
            app_handle,
            pool,
        }
    }
}
