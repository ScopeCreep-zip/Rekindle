use std::sync::Arc;
use std::time::Duration;

use tauri::Emitter;
use tokio::sync::mpsc;

use crate::channels::PresenceEvent;
use crate::db::DbPool;
use crate::state::{AppState, GameDetectorHandle, GameInfoState, SharedState, UserStatus};
use crate::state_helpers;

/// Start the game detection polling loop.
///
/// Runs the `GameDetector` at regular intervals and:
/// 1. Updates `AppState` with current game info
/// 2. Publishes game status to DHT profile subkey 4
/// 3. Emits presence event to frontend
/// 4. Fans out presence updates to all joined communities
pub async fn start_game_detection(
    app_handle: tauri::AppHandle,
    state: Arc<AppState>,
    pool: DbPool,
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
                    let now_ms = rekindle_utils::timestamp_ms();
                    let game_info = detected.as_ref().map(|g| GameInfoState {
                        game_id: g.game_id,
                        game_name: g.game_name.clone(),
                        server_info: g.rich_presence.as_ref().and_then(|rp| rp.details.clone()),
                        elapsed_seconds: u32::try_from((now_ms.saturating_sub(g.started_at_epoch_ms)) / 1000).unwrap_or(u32::MAX),
                        server_address: g.rich_presence.as_ref().and_then(rekindle_game_detect::rich_presence::RichPresence::server_address),
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
                        server_address: game_info.as_ref().and_then(|g| g.server_address.clone()),
                    };
                    let _ = app_handle.emit("presence-event", &event);

                    // Publish game info to DHT profile subkey 4
                    let game_bytes = serde_json::to_vec(&game_info).unwrap_or_default();
                    if let Err(e) = super::message_service::push_profile_update(&state, 4, game_bytes).await {
                        tracing::warn!(error = %e, "failed to publish game info to DHT");
                    }

                    // Fan out presence update to all joined communities
                    fan_out_community_presence(&state, &pool, game_info.as_ref());

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

/// Send `UpdatePresence` to every joined community via coordinator so members see our game status.
fn fan_out_community_presence(
    state: &SharedState,
    _pool: &DbPool,
    game_info: Option<&GameInfoState>,
) {
    let community_ids: Vec<String> = {
        let communities = state.communities.read();
        communities.keys().cloned().collect()
    };

    if community_ids.is_empty() {
        return;
    }

    let status = if game_info.is_some() {
        "online".to_string()
    } else {
        let user_status = state_helpers::identity_status(state).unwrap_or(UserStatus::Online);
        match user_status {
            UserStatus::Online => "online",
            UserStatus::Away => "away",
            UserStatus::Busy => "busy",
            UserStatus::Offline | UserStatus::Invisible => "offline",
        }.to_string()
    };

    for community_id in community_ids {
        let game_info_for_envelope = game_info.map(|g| {
            rekindle_protocol::dht::community::envelope::PresenceGameInfo {
                game_name: g.game_name.clone(),
                game_id: Some(g.game_id),
                elapsed_seconds: Some(g.elapsed_seconds),
                server_address: g.server_address.clone(),
            }
        });

        let pseudonym_key = {
            let communities = state.communities.read();
            communities
                .get(&community_id)
                .and_then(|c| c.my_pseudonym_key.clone())
                .unwrap_or_default()
        };

        // Fire-and-forget — don't block detection loop on network calls
        let s = Arc::clone(state);
        let cid = community_id.clone();
        let status_clone = status.clone();
        tokio::spawn(async move {
            if let Err(e) = crate::commands::community::send_to_coordinator(
                &s,
                &cid,
                rekindle_protocol::dht::community::envelope::CommunityEnvelope::PresenceUpdate {
                    pseudonym_key,
                    status: status_clone,
                    game_info: game_info_for_envelope,
                    route_blob: crate::state_helpers::our_route_blob(&s),
                },
            )
            .await
            {
                tracing::debug!(community = %cid, error = %e, "failed to fan out game presence");
            }
        });
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
