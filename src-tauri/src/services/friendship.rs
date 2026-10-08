//! Phase 7 + Phase 12 — friendship inbox-scan glue.
//!
//! Owns the `FriendshipHandle` struct held on `AppState` plus
//! `spawn_coordinator`, the Tauri-runtime entry point that creates the
//! coordinator channels, installs the `WatchTrigger`'s sender, and
//! spawns the coordinator task.
//!
//! The coordinator + `WatchTrigger` + `VeilidScannerDeps` trait live in
//! `rekindle-friendship`. The `VeilidScannerDeps` impl lives next door
//! in `friendship_deps.rs`. This file is the only place where those
//! pieces meet `AppState` + `tauri::AppHandle`.
//!
//! Plan reference: § Phase 7 + § Phase 12 of
//! `/Users/kali/.claude/plans/memoized-dazzling-torvalds.md`.

use std::sync::Arc;

use parking_lot::Mutex;
use rekindle_friendship::{InboxScanCoordinator, VeilidInboxScanner, WatchTrigger};
use rekindle_lifecycle::{ScopeClosed, SessionScope};
use tokio::sync::{mpsc, watch};

use super::friendship_deps::ScannerDeps;
use crate::state::AppState;

/// Adapter held on `AppState` that owns the coordinator's trigger
/// senders plus the `WatchTrigger`. The coordinator itself stops with the
/// login scope. Replaces the five separate
/// friendship fields that used to live directly on `AppState`.
pub struct FriendshipHandle {
    watch_trigger: Arc<WatchTrigger>,
    direct_tx: Mutex<Option<mpsc::Sender<()>>>,
}

impl FriendshipHandle {
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            watch_trigger: Arc::new(WatchTrigger::new()),
            direct_tx: Mutex::new(None),
        })
    }

    /// Trigger an immediate scan. No-op (debug-trace) if no coordinator
    /// is running. Used by the `friendship_scan_now` Tauri command.
    pub async fn scan_now(&self) {
        let tx = self.direct_tx.lock().clone();
        if let Some(tx) = tx {
            if let Err(e) = tx.send(()).await {
                tracing::warn!(error = %e, "friendship direct trigger send failed");
            }
        } else {
            tracing::debug!("friendship coordinator not running — scan_now no-op");
        }
    }

    /// Dev-only: disable the watch tier for the given duration. While
    /// the deadline is in the future, `fire_watch_trigger` no-ops; only
    /// the 30-second poll backstop + direct triggers deliver scans.
    #[cfg(debug_assertions)]
    pub fn dev_disable_watch(&self, duration: std::time::Duration) {
        self.watch_trigger.disable_for(duration);
    }

    /// Fire the watch-tier trigger. Called by the Veilid DHT dispatch
    /// path when a ValueChange arrives on the user's mailbox key.
    pub fn fire_watch_trigger(&self) {
        self.watch_trigger.fire();
    }

    /// Drop the direct-trigger sender and clear the watch trigger's
    /// sender + dev-disable deadline, so triggers fired while logged out
    /// reach no coordinator. Called by `logout_cleanup`; the coordinator
    /// task itself already stopped with the login scope.
    pub fn clear(&self) {
        *self.direct_tx.lock() = None;
        self.watch_trigger.clear();
    }
}

/// Spawn the inbox-scan coordinator for the current identity on the
/// login scope. Wires the trigger channels into
/// `state.friendship_handle` so Tauri commands (scan_now,
/// dev_disable_watch) and the DHT dispatch path (fire_watch_trigger) can
/// drive them.
///
/// # Errors
/// [`ScopeClosed`] when the session ended before the coordinator started.
pub fn spawn_coordinator(
    state: &Arc<AppState>,
    app_handle: tauri::AppHandle,
    scope: &Arc<SessionScope>,
) -> Result<(), ScopeClosed> {
    let handle = Arc::clone(&state.friendship_handle);
    handle.clear();

    let (direct_tx, direct_rx) = mpsc::channel(8);
    let (watch_tx, watch_rx) = watch::channel(0u64);

    *handle.direct_tx.lock() = Some(direct_tx);
    handle.watch_trigger.install_sender(watch_tx);

    let deps = Arc::new(ScannerDeps {
        state: Arc::clone(state),
        app_handle,
    });
    let scanner = Arc::new(VeilidInboxScanner::new(deps));
    let coord = InboxScanCoordinator::new(scanner, direct_rx, watch_rx);
    scope.spawn_with_token("friendship inbox coordinator", |stop| coord.run(stop))
}
