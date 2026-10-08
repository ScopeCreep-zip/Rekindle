//! Phase 12 — `VeilidScannerDeps` impl for the friendship inbox-scan
//! coordinator.
//!
//! Keeps `Arc<AppState>` + `tauri::AppHandle` confined to src-tauri;
//! the crate's `VeilidInboxScanner<D>` is monomorphised over this type
//! at `spawn_coordinator` time.

use std::sync::Arc;

use async_trait::async_trait;
use rekindle_friendship::{ScanError, VeilidScannerDeps};

use crate::state::AppState;

pub(super) struct ScannerDeps {
    pub(super) state: Arc<AppState>,
    pub(super) app_handle: tauri::AppHandle,
}

#[async_trait]
impl VeilidScannerDeps for ScannerDeps {
    async fn sync_friends_now(&self) -> Result<u32, ScanError> {
        // sync_friends_now refreshes friend DHT records (status, prekey,
        // route) which is also where a peer's pending-request signal
        // arrives. Returns Result<(), String>; map errors through. The
        // scanner trait expects a processed count for diagnostics —
        // sync_friends_now doesn't return one, so we report 0 (the
        // tracing inside sync_friends already logs per-friend results).
        crate::services::sync_service::sync_friends_now(&self.state, &self.app_handle)
            .await
            .map_err(ScanError::InboxUnavailable)?;
        Ok(0)
    }
}
