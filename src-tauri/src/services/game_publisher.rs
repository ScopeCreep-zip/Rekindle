//! Phase 12 — `GameDetectorPublisher` impl + community presence fan-out.
//!
//! Bundles the four per-change side-effects the
//! `rekindle_game_detect::runtime::run` loop produces on each
//! state-change:
//!
//! 1. Update the `AppState.game_detector.current_game` cache (read by
//!    the `get_game_status` Tauri command).
//! 2. Emit `PresenceEvent::GameChanged` on the `presence-event` channel.
//! 3. Publish to DHT profile subkey 4.
//! 4. Fan out `PresenceUpdate` to every joined community via gossip mesh.
//!
//! Plan reference: § Phase 12 of
//! `/Users/kali/.claude/plans/memoized-dazzling-torvalds.md`.

use std::sync::Arc;

use async_trait::async_trait;
use rekindle_game_detect::{DetectedGame, GameDetectorPublisher};

use crate::channels::PresenceEvent;
use crate::state::{AppState, GameInfoState, SharedState, UserStatus};
use crate::state_helpers;

pub struct GamePublisher {
    pub app_handle: tauri::AppHandle,
    pub state: Arc<AppState>,
}

#[async_trait]
impl GameDetectorPublisher for GamePublisher {
    async fn publish_game_status(&self, detected: Option<DetectedGame>) {
        let now_ms = rekindle_utils::timestamp_ms();
        let game_info = detected.as_ref().map(|g| GameInfoState {
            game_id: g.game_id,
            game_name: g.game_name.clone(),
            server_info: g.rich_presence.as_ref().and_then(|rp| rp.details.clone()),
            elapsed_seconds: u32::try_from(now_ms.saturating_sub(g.started_at_epoch_ms) / 1000)
                .unwrap_or(u32::MAX),
            server_address: g
                .rich_presence
                .as_ref()
                .and_then(rekindle_game_detect::rich_presence::RichPresence::server_address),
        });

        // 1. Update AppState cache (read by get_game_status command).
        {
            let mut gd = self.state.game_detector.lock();
            if let Some(ref mut handle) = *gd {
                handle.current_game.clone_from(&game_info);
            }
        }

        // 2. Emit presence event to frontend.
        let event = PresenceEvent::GameChanged {
            public_key: state_helpers::owner_key_or_default(&self.state),
            game_name: game_info.as_ref().map(|g| g.game_name.clone()),
            game_id: game_info.as_ref().map(|g| g.game_id),
            elapsed_seconds: game_info.as_ref().map(|g| g.elapsed_seconds),
            server_address: game_info.as_ref().and_then(|g| g.server_address.clone()),
        };
        crate::event_dispatch::emit_live(&self.app_handle, "presence-event", &event);

        // 3. Publish game info to DHT profile subkey 4.
        let game_bytes = serde_json::to_vec(&game_info).unwrap_or_default();
        if let Err(e) =
            super::message_service::push_profile_update(&self.state, 4, game_bytes).await
        {
            tracing::warn!(error = %e, "failed to publish game info to DHT");
        }

        // 4. Fan out presence update to all joined communities.
        fan_out_community_presence(&self.state, game_info.as_ref());
    }
}

/// Broadcast `PresenceUpdate` to every joined community via gossip mesh
/// so members see our game status.
fn fan_out_community_presence(state: &SharedState, game_info: Option<&GameInfoState>) {
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
        }
        .to_string()
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

        // Fire-and-forget via gossip mesh — ephemeral presence, no durable relay needed.
        if let Err(e) = crate::services::community::send_to_mesh(
            state,
            &community_id,
            &rekindle_protocol::dht::community::envelope::CommunityEnvelope::PresenceUpdate {
                pseudonym_key,
                status: status.clone(),
                game_info: game_info_for_envelope,
                route_blob: crate::state_helpers::our_route_blob(state),
            },
        ) {
            tracing::debug!(community = %community_id, error = %e, "failed to fan out game presence");
        }
    }
}
