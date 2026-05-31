//! Phase 14.r split — small I/O helpers reached by `deps_impl`.
//!
//! Each helper is a free fn used by exactly one trait method:
//! `restart_audio_devices`, `resolve_peer_route`, `load_member_names`,
//! `broadcast_media_capabilities`.

use std::collections::HashMap;
use std::sync::Arc;

use crate::db::DbPool;
use crate::state::AppState;
use crate::state_helpers;

pub(super) fn restart_audio_devices_impl(state: &AppState) -> Result<(), String> {
    let mut ve = state.voice_engine.lock();
    let handle = ve.as_mut().ok_or("no active voice engine")?;
    handle
        .engine
        .start_capture()
        .map_err(|e| format!("failed to restart capture: {e}"))?;
    handle
        .engine
        .start_playback()
        .map_err(|e| format!("failed to restart playback: {e}"))?;
    Ok(())
}

pub(super) async fn resolve_peer_route_impl(
    state: &Arc<AppState>,
    peer_pubkey_hex: &str,
) -> Option<Vec<u8>> {
    if let Some(blob) = state_helpers::cached_route_blob(state, peer_pubkey_hex) {
        return Some(blob);
    }
    if let Some(blob) =
        crate::services::message_service::try_fetch_route_from_dht(state, peer_pubkey_hex).await
    {
        return Some(blob);
    }
    None
}

pub(super) fn broadcast_media_capabilities_impl(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
) -> Result<(), String> {
    use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
    let caps = rekindle_video::MediaCapabilities::interim_default();
    let envelope = CommunityEnvelope::Control(ControlPayload::MediaCapabilities {
        channel_id: channel_id.to_string(),
        max_pixel_count: caps.max_pixel_count,
        max_fps: caps.max_fps,
        codecs: caps.codecs,
    });
    crate::services::community::send_to_mesh(state, community_id, &envelope)
}

pub(super) fn log_voice_membership_impl(
    state: &Arc<AppState>,
    pool: &DbPool,
    community_id: &str,
    channel_id: &str,
    joined: bool,
) {
    let owner = state_helpers::owner_key_or_default(state);
    let pseudo = state
        .communities
        .read()
        .get(community_id)
        .and_then(|c| c.my_pseudonym_key.clone())
        .unwrap_or_default();
    if joined {
        crate::services::community::analytics::log_voice_join(
            pool,
            &owner,
            community_id,
            channel_id,
            &pseudo,
        );
    } else {
        crate::services::community::analytics::log_voice_leave(
            pool,
            &owner,
            community_id,
            channel_id,
            &pseudo,
        );
    }
}

pub(super) async fn load_community_member_names_impl(
    state: &Arc<AppState>,
    community_id: Option<&str>,
) -> HashMap<String, String> {
    let Some(cid) = community_id else {
        return HashMap::new();
    };
    let app_handle = state.app_handle.read().clone();
    let Some(ref ah) = app_handle else {
        return HashMap::new();
    };
    let Some(pool) = tauri::Manager::try_state::<DbPool>(ah) else {
        return HashMap::new();
    };
    let cid_owned = cid.to_string();
    crate::db_helpers::db_call(&pool, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT pseudonym_key, display_name FROM community_members WHERE community_id = ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![cid_owned], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        Ok(rows.filter_map(Result::ok).collect())
    })
    .await
    .unwrap_or_default()
}
