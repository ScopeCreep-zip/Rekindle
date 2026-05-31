//! Phase 23.D.4 — `handle_message_notification` extracted from
//! `message_notifications.rs` to keep that file under the 500-LoC
//! cap (Invariant 1). Orchestrates the announce → fetch → verify →
//! decrypt → persist → automod gate → emit pipeline for an inbound
//! channel-message notification.

use std::sync::Arc;

use tauri::Manager;

use rekindle_records::retry;

use crate::db::DbPool;
use crate::db_helpers::db_fire;
use crate::state::AppState;
use crate::state_helpers;

use super::message_notifications::{
    decrypt_message_body, emit_automod_alert, emit_message_received, fetch_channel_message,
    message_exists, queue_message_fetch_retry, update_peer_sequence, verify_notification_message,
    PendingMessageFetch,
};

pub async fn handle_message_notification(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pending: PendingMessageFetch,
) -> Result<(), String> {
    let pool: tauri::State<'_, DbPool> = app_handle.state();
    let owner_key = state_helpers::owner_key_or_default(state);
    if owner_key.is_empty() {
        return Err("owner key unavailable".into());
    }
    if message_exists(pool.inner(), &owner_key, &pending.message_id).await {
        return Ok(());
    }

    let fetched = match fetch_channel_message(
        state,
        &pending.community_id,
        &pending.channel_id,
        pending.subkey_index,
        &pending.message_id,
    )
    .await
    {
        Ok(fetched) => fetched,
        Err(error) => {
            if pending.attempt + 1 < retry::MAX_RETRIES {
                queue_message_fetch_retry(state.clone(), pending);
            }
            return Err(error);
        }
    };
    let message = fetched.message;
    let forwarded_from_author = fetched.forwarded_from_author;

    if let Err(error) = verify_notification_message(&pending, &message) {
        if pending.attempt + 1 < retry::MAX_RETRIES {
            queue_message_fetch_retry(state.clone(), pending.clone());
        }
        return Err(error.into());
    }

    // M10.4 — receiver-side slowmode (architecture §28.7 line 3187).
    // Sender-side enforcement is advisory; modified clients can ignore
    // their own slowmode UI. Honest receivers drop sub-window writes
    // unless the sender holds BYPASS_SLOWMODE (bit 29).
    if !crate::services::community::receiver_limits::check_slowmode(
        state,
        &pending.community_id,
        &pending.channel_id,
        &message.sender_pseudonym,
        rekindle_utils::timestamp_secs(),
    ) {
        tracing::trace!(
            community = %pending.community_id,
            channel = %pending.channel_id,
            sender = %message.sender_pseudonym,
            "slowmode floor exceeded — dropping silently"
        );
        return Ok(());
    }

    if !state_helpers::merge_lamport(state, &pending.community_id, message.lamport_ts) {
        // M9.2 — sender's claimed Lamport is too far ahead of our
        // local clock. Drop the message: a forged-future timestamp
        // from a malicious peer must not fast-forward our clock.
        tracing::trace!(
            community = %pending.community_id,
            channel = %pending.channel_id,
            sender = %message.sender_pseudonym,
            received_lamport = message.lamport_ts,
            "lamport drift cap exceeded — dropping message silently"
        );
        return Ok(());
    }
    let Some(body) = decrypt_message_body(
        app_handle,
        state,
        &pending.community_id,
        &pending.channel_id,
        &pending,
        &message,
    ) else {
        let requester_pseudonym = {
            let communities = state.communities.read();
            communities
                .get(&pending.community_id)
                .and_then(|community| community.my_pseudonym_key.clone())
        };
        if let Some(requester_pseudonym) = requester_pseudonym {
            // A3/P1.3 — spawn a retry loop with cascade fall-through instead of
            // a single fire-and-forget send. The previous send dropped silently
            // if the deterministic responder was offline, leaving the message
            // permanently undecryptable until a future rotation broadcast.
            crate::services::community::spawn_mek_request_with_retry(
                state.clone(),
                pending.community_id.clone(),
                pending.channel_id.clone(),
                message.mek_generation,
                requester_pseudonym,
            );
        }
        emit_message_received(
            app_handle,
            state,
            &pending,
            message.sender_pseudonym.clone(),
            String::new(),
            message.timestamp,
            true,
            false,
        );
        update_peer_sequence(
            state,
            &pending.community_id,
            &message.sender_pseudonym,
            &pending.channel_id,
            pending.sequence,
        );
        return Ok(());
    };

    let automod_action =
        crate::services::community::automod::evaluate_message(state, &pending.community_id, &body)
            .unwrap_or(crate::services::community::automod::AutoModAction::Allow);
    if automod_action == crate::services::community::automod::AutoModAction::BlockLocally {
        tracing::info!(
            community_id = %pending.community_id,
            channel_id = %pending.channel_id,
            message_id = %pending.message_id,
            "automod blocked"
        );
        update_peer_sequence(
            state,
            &pending.community_id,
            &message.sender_pseudonym,
            &pending.channel_id,
            pending.sequence,
        );
        return Ok(());
    }

    let message_id = pending.message_id.clone();
    let channel_id = pending.channel_id.clone();
    let sender = message.sender_pseudonym.clone();
    let body_for_db = body.clone();
    let timestamp = i64::try_from(message.timestamp).unwrap_or(i64::MAX);
    let mek_generation = i64::try_from(message.mek_generation).unwrap_or(i64::MAX);
    let lamport_ts = message.lamport_ts;
    let automod_blurred =
        automod_action == crate::services::community::automod::AutoModAction::BlurContent;
    let forwarded_from_author_for_db = forwarded_from_author.clone();
    let attachment_json_for_db: Option<String> = message.attachment.as_ref().map(|att| {
        serde_json::to_string(&serde_json::json!({
            "attachmentId": hex::encode(att.attachment_id),
            "filename": att.filename,
            "mimeType": att.mime_type,
            "totalSize": att.total_size,
            "chunkCount": att.chunk_count,
            "localPath": serde_json::Value::Null,
        }))
        .unwrap_or_default()
    });
    let flags_for_db = message.flags;
    db_fire(
        pool.inner(),
        "store notified channel message",
        move |conn| {
            crate::message_repo::insert_channel_message_full(
                conn,
                &owner_key,
                &channel_id,
                &sender,
                &body_for_db,
                timestamp,
                false,
                Some(mek_generation),
                &message_id,
                lamport_ts,
                automod_blurred,
                forwarded_from_author_for_db.as_deref(),
                flags_for_db,
                attachment_json_for_db.as_deref(),
            )
        },
    );

    update_peer_sequence(
        state,
        &pending.community_id,
        &message.sender_pseudonym,
        &pending.channel_id,
        pending.sequence,
    );

    let sender_pseudonym = message.sender_pseudonym.clone();
    emit_message_received(
        app_handle,
        state,
        &pending,
        sender_pseudonym.clone(),
        body.clone(),
        message.timestamp,
        false,
        automod_blurred,
    );

    if automod_action == crate::services::community::automod::AutoModAction::AlertModerators {
        let rule_name =
            crate::services::community::automod::list_rules(state, &pending.community_id)
                .ok()
                .and_then(|rules| {
                    rules
                        .into_iter()
                        .find(|rule| {
                            rule.enabled
                                && rule.action == "alert_moderators"
                                && (rule.keywords.iter().any(|keyword| {
                                    body.to_lowercase().contains(&keyword.to_lowercase())
                                }) || rule.regex_patterns.iter().any(|pattern| {
                                    regex::Regex::new(pattern)
                                        .map(|compiled| compiled.is_match(&body))
                                        .unwrap_or(false)
                                }))
                        })
                        .map(|rule| rule.name)
                })
                .unwrap_or_else(|| "AutoMod".to_string());
        emit_automod_alert(
            app_handle,
            state,
            &pending.community_id,
            &pending.channel_id,
            &pending.message_id,
            &rule_name,
        );
    }

    // Architecture §28.5 line 3120 — pre-decryption notification
    // routing uses the cleartext mention metadata the sender stamped
    // on the envelope. Body parsing is now reserved for tests / legacy
    // payloads that never carried these fields.
    if crate::services::community::should_emit_message_notification(
        state,
        pool.inner(),
        &pending.community_id,
        &pending.channel_id,
        &message.sender_pseudonym,
        crate::services::community::notifications::CleartextMentions {
            mentioned_pseudonyms: &message.mentioned_pseudonyms,
            mentioned_roles: &message.mentioned_roles,
            flags: message.flags,
        },
    )
    .await
    .unwrap_or(false)
    {
        crate::services::community::emit_message_notification(
            app_handle,
            state,
            pool.inner(),
            &pending.community_id,
            &pending.channel_id,
            &sender_pseudonym,
            &body,
        )
        .await;
    }

    Ok(())
}

#[cfg(test)]
#[path = "message_notifications_tests.rs"]
mod tests;
