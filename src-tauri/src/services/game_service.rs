//! Phase 3 + Phase 12 — game-detection runtime entry points.
//!
//! Owns `initialize` (installs the game-detector handle on AppState)
//! and `start_game_detection` (builds the publisher + detector and runs
//! the loop on the login scope until the session ends). The publisher impl +
//! community fan-out live in `game_publisher.rs`; the 30-second poll
//! loop + change detection live in `rekindle-game-detect::runtime`.
//!
//! Plan reference: § Phase 3 + § Phase 12 of
//! `/Users/kali/.claude/plans/memoized-dazzling-torvalds.md`.

use std::sync::Arc;

use rekindle_game_detect::{GameDatabase, GameDetector, DEFAULT_POLL_INTERVAL};
use tokio_util::sync::CancellationToken;

use crate::state::{AppState, GameDetectorHandle};

/// Install a fresh game detector handle (no game yet) for this login.
pub fn initialize(state: &AppState) {
    *state.game_detector.lock() = Some(GameDetectorHandle { current_game: None });
}

/// Start the game-detection runtime. Constructs the detector +
/// publisher and runs the loop until `stop` is cancelled.
pub async fn start_game_detection(
    app_handle: tauri::AppHandle,
    state: Arc<AppState>,
    stop: CancellationToken,
) {
    let publisher = Arc::new(super::game_publisher::GamePublisher { app_handle, state });
    let detector = GameDetector::new(GameDatabase::bundled(), DEFAULT_POLL_INTERVAL);
    rekindle_game_detect::run_runtime(detector, publisher, stop, DEFAULT_POLL_INTERVAL).await;
}
