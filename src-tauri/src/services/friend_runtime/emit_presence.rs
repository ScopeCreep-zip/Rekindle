//! Phase 23.C — emit_friends_presence orchestrator lifted from
//! `commands/friends.rs`. Waits up to 15s for Veilid network readiness,
//! kicks a best-effort DHT sync, then re-emits FriendOnline +
//! StatusChanged presence events for every accepted, non-offline friend
//! so frontend listeners registered after hydration receive current
//! presence state.

use std::sync::Arc;

use crate::services;
use crate::state::{AppState, FriendshipState, UserStatus};

pub async fn emit_friends_presence_inner(
    state: Arc<AppState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let mut rx = state.network_ready_rx.clone();
    let _ready = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            if *rx.borrow_and_update() {
                return true;
            }
            if rx.changed().await.is_err() {
                return false;
            }
        }
    })
    .await
    .unwrap_or(false);

    let _ = services::sync_service::sync_friends_now(&state, &app).await;

    let friends: Vec<(String, UserStatus)> = {
        let friends = state.friends.read();
        friends
            .values()
            .filter(|f| f.friendship_state == FriendshipState::Accepted)
            .map(|f| (f.public_key.clone(), f.status))
            .collect()
    };
    for (key, status) in friends {
        if status != UserStatus::Offline {
            crate::event_dispatch::emit_live(
                &app,
                "presence-event",
                &crate::channels::PresenceEvent::FriendOnline {
                    public_key: key.clone(),
                },
            );
            crate::event_dispatch::emit_live(
                &app,
                "presence-event",
                &crate::channels::PresenceEvent::StatusChanged {
                    public_key: key,
                    status: format!("{status:?}").to_lowercase(),
                    status_message: None,
                },
            );
        }
    }
    Ok(())
}
