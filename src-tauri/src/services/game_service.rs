//! Phase 3 + Phase 12 — game-detection runtime entry points.
//!
//! Owns `initialize` (writes the shutdown_tx onto AppState so
//! logout_cleanup can signal exit) and `start_game_detection` (builds
//! the publisher + detector and runs the loop). The publisher impl +
//! community fan-out live in `game_publisher.rs`; the 30-second poll
//! loop + change detection live in `rekindle-game-detect::runtime`.
//!
//! Plan reference: § Phase 3 + § Phase 12 of
//! `/Users/kali/.claude/plans/memoized-dazzling-torvalds.md`.

use std::sync::Arc;

use rekindle_game_detect::{DEFAULT_POLL_INTERVAL, GameDatabase, GameDetector};
use tokio::sync::mpsc;

use crate::db::DbPool;
use crate::state::{AppState, GameDetectorHandle};

/// Initialize the game detector handle in `AppState`. Stores the
/// shutdown_tx so `logout_cleanup` can signal the runtime to exit.
pub fn initialize(state: &AppState, shutdown_tx: mpsc::Sender<()>) {
    let handle = GameDetectorHandle {
        shutdown_tx,
        current_game: None,
    };
    *state.game_detector.lock() = Some(handle);
}

/// Start the game-detection runtime. Constructs the detector +
/// publisher and runs the loop until shutdown.
pub async fn start_game_detection(
    app_handle: tauri::AppHandle,
    state: Arc<AppState>,
    _pool: DbPool,
    shutdown_rx: mpsc::Receiver<()>,
) {
    // `_pool` accepted for call-site compatibility; presence updates
    // flow through gossip mesh, not SQLite.
    let publisher = Arc::new(super::game_publisher::GamePublisher { app_handle, state });
    let detector = GameDetector::new(GameDatabase::bundled(), DEFAULT_POLL_INTERVAL);
    rekindle_game_detect::run_runtime(detector, publisher, shutdown_rx, DEFAULT_POLL_INTERVAL).await;
}
