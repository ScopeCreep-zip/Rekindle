//! Architecture §28.8 — link preview orchestration.
//!
//! Sender side: fetch OpenGraph metadata via `rekindle-link-preview`,
//! broadcast a `ControlPayload::LinkPreview` to the community mesh.
//! Receiver side: gate on the sender's `EMBED_LINKS` permission, then
//! emit a `community-event` so the UI renders inline.

use std::sync::Arc;

use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_protocol::dht::community::permissions_v2::Permissions;
use rekindle_types::link_preview::LinkPreview;
use tauri::{Emitter as _, Manager};

use crate::channels::CommunityEvent;
use crate::commands::community::require_permission;
use crate::db::DbPool;
use crate::db_helpers::db_call_or_default;
use crate::state::{AppState, SharedState};
use crate::state_helpers;

/// Sender side: fetch the OpenGraph payload, then broadcast it via
/// gossip alongside the original message.
pub async fn fetch_and_broadcast(
    state: &SharedState,
    community_id: &str,
    channel_id: &str,
    message_id: &str,
    url: &str,
) -> Result<LinkPreview, String> {
    require_permission(state, community_id, Permissions::EMBED_LINKS)?;
    // Architecture §28.8 line 3220 — respect the user's IP-privacy
    // preference. When the toggle is off, the OpenGraph fetch is
    // skipped entirely so no third-party server learns this device's IP.
    if !user_link_previews_enabled(state).await {
        return Err("link preview generation disabled in settings".to_string());
    }
    let preview = rekindle_link_preview::fetch_link_preview(url, message_id)
        .await
        .map_err(|e| format!("link preview fetch failed: {e}"))?;
    let envelope = CommunityEnvelope::Control(ControlPayload::LinkPreview {
        channel_id: channel_id.to_string(),
        message_id: preview.message_id.clone(),
        url: preview.url.clone(),
        title: preview.title.clone(),
        description: preview.description.clone(),
        image_url: preview.image_url.clone(),
        site_name: preview.site_name.clone(),
        fetched_at: preview.fetched_at,
    });
    crate::services::community::send_to_mesh(state, community_id, &envelope)?;
    Ok(preview)
}

/// Receiver side: trust-gate the incoming preview, then emit it to the UI.
pub fn handle_incoming_link_preview(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    channel_id: String,
    message_id: String,
    url: String,
    title: Option<String>,
    description: Option<String>,
    image_url: Option<String>,
    site_name: Option<String>,
    fetched_at: u64,
) {
    if !sender_has_embed_links(state, community_id, sender_pseudonym) {
        tracing::debug!(
            community = %community_id,
            sender = %sender_pseudonym,
            "dropping LinkPreview from sender without EMBED_LINKS"
        );
        return;
    }
    let _ = app_handle.emit(
        "community-event",
        CommunityEvent::LinkPreviewReceived {
            community_id: community_id.to_string(),
            sender_pseudonym: sender_pseudonym.to_string(),
            channel_id,
            message_id,
            url,
            title,
            description,
            image_url,
            site_name,
            fetched_at,
        },
    );
}

async fn user_link_previews_enabled(state: &SharedState) -> bool {
    let Ok(owner_key) = state_helpers::current_owner_key(state) else {
        return true;
    };
    let Some(app_handle) = state_helpers::app_handle(state) else {
        return true;
    };
    let pool: tauri::State<'_, DbPool> = app_handle.state();
    db_call_or_default(pool.inner(), move |conn| {
        let value: Option<i64> = conn
            .query_row(
                "SELECT link_previews_enabled FROM app_settings WHERE owner_key = ?1",
                rusqlite::params![owner_key],
                |row| row.get(0),
            )
            .ok();
        Ok(value.unwrap_or(1) != 0)
    })
    .await
}

fn sender_has_embed_links(
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym_hex: &str,
) -> bool {
    use rekindle_governance::permissions::compute_permissions;
    use rekindle_types::id::PseudonymKey;

    let communities = state.communities.read();
    let Some(community) = communities.get(community_id) else {
        return false;
    };
    let Some(gov) = community.governance_state.as_ref() else {
        return false;
    };
    let Ok(pk_bytes) = hex::decode(sender_pseudonym_hex) else {
        return false;
    };
    let Ok(pk_arr) = <[u8; 32]>::try_from(pk_bytes.as_slice()) else {
        return false;
    };
    let pseudonym = PseudonymKey(pk_arr);
    let perms = compute_permissions(&pseudonym, None, gov, rekindle_utils::timestamp_secs());
    Permissions::from_bits_truncate(perms).contains(Permissions::EMBED_LINKS)
}
