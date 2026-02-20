use std::sync::Arc;
use std::time::Duration;

use tauri::Emitter;
use tokio::sync::mpsc;

use crate::channels::PresenceEvent;
use crate::state::{AppState, GameDetectorHandle, GameInfoState};
use crate::state_helpers;

/// Start the game detection polling loop.
///
/// Runs the `GameDetector` at regular intervals and:
/// 1. Updates `AppState` with current game info
/// 2. Publishes game status to DHT profile subkey 4
/// 3. Emits presence event to frontend
pub async fn start_game_detection(
    app_handle: tauri::AppHandle,
    state: Arc<AppState>,
    mut shutdown_rx: mpsc::Receiver<()>,
) {
    tracing::info!("game detection service started");

    let database = rekindle_game_detect::GameDatabase::bundled();
    let mut detector = rekindle_game_detect::GameDetector::new(database, Duration::from_secs(30));

    let mut last_game: Option<String> = None;

    let mut interval = tokio::time::interval(Duration::from_secs(30));

    loop {
        tokio::select! {
            _ = interval.tick() => {
                let detected = detector.scan_once();

                let current_name = detected.as_ref().map(|g| g.game_name.clone());

                // Only emit events when game state changes
                if current_name != last_game {
                    let now_ms = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis().try_into().unwrap_or(u64::MAX);
                    let game_info = detected.as_ref().map(|g| GameInfoState {
                        game_id: g.game_id,
                        game_name: g.game_name.clone(),
                        server_info: None,
                        elapsed_seconds: u32::try_from((now_ms.saturating_sub(g.started_at_epoch_ms)) / 1000).unwrap_or(u32::MAX),
                    });

                    // Update AppState
                    {
                        let mut gd = state.game_detector.lock();
                        if let Some(ref mut handle) = *gd {
                            handle.current_game.clone_from(&game_info);
                        }
                    }

                    // Emit presence event to frontend
                    let event = PresenceEvent::GameChanged {
                        public_key: state_helpers::owner_key_or_default(&state),
                        game_name: game_info.as_ref().map(|g| g.game_name.clone()),
                        game_id: game_info.as_ref().map(|g| g.game_id),
                        elapsed_seconds: game_info.as_ref().map(|g| g.elapsed_seconds),
                    };
                    let _ = app_handle.emit("presence-event", &event);

                    // Publish game info to DHT profile subkey 4
                    let game_bytes = serde_json::to_vec(&game_info).unwrap_or_default();
                    if let Err(e) = super::message_service::push_profile_update(&state, 4, game_bytes).await {
                        tracing::warn!(error = %e, "failed to publish game info to DHT");
                    }

                    if let Some(ref name) = current_name {
                        tracing::info!(game = %name, "game detected");
                    } else {
                        tracing::info!("game ended");
                    }

                    last_game = current_name;
                }
            }
            _ = shutdown_rx.recv() => {
                tracing::info!("game detection service shutting down");
                break;
            }
        }
    }
}

/// Initialize the game detector handle in `AppState`.
pub fn initialize(state: &AppState, shutdown_tx: mpsc::Sender<()>) {
    let handle = GameDetectorHandle {
        shutdown_tx,
        current_game: None,
    };
    *state.game_detector.lock() = Some(handle);
}
