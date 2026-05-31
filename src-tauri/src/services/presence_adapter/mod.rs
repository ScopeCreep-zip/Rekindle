//! Phase 21 REDO — friend + community presence adapter.
//!
//! Implements `rekindle_presence::{FriendPresenceDeps,
//! CommunityPresenceDeps}` against the live AppState + AppHandle +
//! DbPool. Split into per-trait files (Invariant 1 ≤500 LoC):
//!
//! - `friend_deps.rs` — `FriendPresenceDeps` impl (20 methods)
//! - `community_deps.rs` — `CommunityPresenceDeps` impl (21 methods)
//! - `mapping.rs` — `UserStatusKind` ↔ `UserStatus` + game-info +
//!   event mapping helpers shared by both impls
//! - `persist.rs` — SQLite-backed helpers (channel-log catchup
//!   insert, history-range MIN/MAX scan, member upsert) extracted
//!   from `community_deps.rs` to keep it under the LoC cap

use std::sync::Arc;

use tauri::AppHandle;
use tauri::Manager as _;

use crate::db::DbPool;
use crate::state::AppState;

mod auto_expand;
pub mod community_deps;
pub mod friend_deps;
mod gossip_overlay;
mod mapping;
mod member_state;
mod pending_sync;
mod persist;
mod scan;
mod state_reads;

/// Adapter struct — same shape as Phase 17/18/19/20 adapters.
pub struct PresenceAdapter {
    pub(super) state: Arc<AppState>,
    pub(super) app_handle: AppHandle,
    pub(super) pool: DbPool,
}

impl PresenceAdapter {
    pub fn new(state: Arc<AppState>, app_handle: AppHandle, pool: DbPool) -> Self {
        Self {
            state,
            app_handle,
            pool,
        }
    }
}

/// Build a one-shot adapter from the live AppState. The facades in
/// `services/presence_service.rs` + `services/community/presence/*`
/// construct one per call (cheap — just clones three Arcs) and hand
/// it to the crate's orchestrators.
pub fn build_adapter(state: &Arc<AppState>) -> Option<PresenceAdapter> {
    let app_handle = state.app_handle.read().clone()?;
    let pool = app_handle.try_state::<DbPool>()?.inner().clone();
    Some(PresenceAdapter::new(Arc::clone(state), app_handle, pool))
}
