//! Architecture §28.8 — link preview orchestration.
//!
//! Sender side: fetch OpenGraph metadata via `rekindle-link-preview`,
//! broadcast a `ControlPayload::LinkPreview` to the community mesh.
//! Receiver side: gate on the sender's `EMBED_LINKS` permission, the
//! shared `accept_inbound` policy and the message's author, then emit a
//! `community-event` so the UI renders inline.

use std::sync::Arc;

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_types::link_preview::LinkPreview;
use rekindle_types::permissions;

use crate::channels::CommunityEvent;
use crate::commands::community::require_permission;
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
    require_permission(state, community_id, permissions::EMBED_LINKS)?;
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
        site_name: preview.site_name.clone(),
        fetched_at: preview.fetched_at,
    });
    crate::services::community::send_to_mesh(state, community_id, &envelope)?;
    Ok(preview)
}

/// Receiver side: accept a peer's preview only if
/// - the sender holds `EMBED_LINKS`;
/// - it passes `rekindle_link_preview::accept_inbound` (https, bounded text);
/// - it names a message we hold in that channel, written by the sender.
///
/// The last check stops a member pinning a phishing card onto someone
/// else's message, an admin announcement included.
pub fn handle_incoming_link_preview(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    channel_id: String,
    preview: LinkPreview,
) {
    if !sender_has_embed_links(state, community_id, sender_pseudonym) {
        tracing::debug!(
            community = %community_id,
            sender = %sender_pseudonym,
            "dropping LinkPreview from sender without EMBED_LINKS"
        );
        return;
    }
    let Some(preview) = rekindle_link_preview::accept_inbound(preview) else {
        tracing::debug!(community = %community_id, "dropping LinkPreview with a non-https URL");
        return;
    };
    let Ok(owner_key) = state_helpers::current_owner_key(state) else {
        return;
    };
    let app = app_handle.clone();
    let community_id = community_id.to_owned();
    let sender = sender_pseudonym.to_owned();
    let Ok(pool) = state.db.current() else {
        return;
    };
    crate::state_helpers::login_scope_or_closed(state).spawn_or_drop(
        "link preview fetch",
        async move {
            let (chan, msg) = (channel_id.clone(), preview.message_id.clone());
            let author: Option<String> = db_call_or_default(&pool, move |conn| {
                conn.query_row(
                    "SELECT sender_key FROM messages \
                 WHERE owner_key = ?1 AND conversation_id = ?2 \
                 AND conversation_type = 'channel' AND message_id = ?3",
                    rusqlite::params![owner_key, chan, msg],
                    |row| row.get(0),
                )
                .map(Some)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })
            })
            .await;
            if author.as_deref() != Some(sender.as_str()) {
                tracing::debug!(
                    community = %community_id,
                    "dropping LinkPreview not authored by the message's sender"
                );
                return;
            }
            crate::event_dispatch::emit_community(
                &app,
                CommunityEvent::LinkPreviewReceived {
                    community_id,
                    sender_pseudonym: sender,
                    channel_id,
                    message_id: preview.message_id,
                    url: preview.url,
                    title: preview.title,
                    description: preview.description,
                    site_name: preview.site_name,
                    fetched_at: preview.fetched_at,
                },
            );
        },
    );
}

async fn user_link_previews_enabled(state: &SharedState) -> bool {
    let Ok(owner_key) = state_helpers::current_owner_key(state) else {
        return false;
    };
    let Ok(pool) = state.db.current() else {
        return false;
    };
    crate::services::community_link_previews_runtime::link_previews_enabled(&pool, owner_key).await
}

fn sender_has_embed_links(
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym_hex: &str,
) -> bool {
    let perms = state_helpers::permissions_for(state, community_id, sender_pseudonym_hex, None)
        .unwrap_or(0);
    rekindle_governance::permissions::has_capability(
        perms,
        rekindle_types::permissions::EMBED_LINKS,
    )
}
