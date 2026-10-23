//! Phase 23.D.4 — `handle_message_notification` extracted from
//! `message_notifications.rs` to keep that file under the 500-LoC
//! cap (Invariant 1). Orchestrates the announce → fetch → verify → Host
//! ingest (open, D26 gate, store, publish; plan E3.1 F3) → display and
//! notify pipeline for an inbound channel-message notification.

use std::sync::Arc;

use rekindle_node::host::adapters::channel::ingest::{Ingested, ReceivedPost};
use rekindle_records::retry;

use crate::state::AppState;
use crate::state_helpers;

use super::message_notifications::{
    emit_message_received, fetch_channel_message, message_exists, queue_message_fetch_retry,
    update_peer_sequence, verify_notification_message, PendingMessageFetch,
};

pub async fn handle_message_notification(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pending: PendingMessageFetch,
) -> Result<(), String> {
    let pool = state.db.current()?;
    let owner_key = state_helpers::owner_key_or_default(state);
    if owner_key.is_empty() {
        return Err("owner key unavailable".into());
    }
    if message_exists(&pool, &owner_key, &pending.message_id).await? {
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
    let message = fetched.message.clone();

    if let Err(error) = verify_notification_message(&pending, &message) {
        if pending.attempt + 1 < retry::MAX_RETRIES {
            queue_message_fetch_retry(state.clone(), pending.clone());
        }
        return Err(error.into());
    }

    // M9.2 — the clock advances past the message's timestamp by at most
    // MAX_LAMPORT_DRIFT + 1; the message itself is kept and orders by its
    // own timestamp (a clamp, not a drop — a fresh restart's clock may sit
    // far below live traffic).
    state_helpers::merge_message_lamport(state, &pending.community_id, message.lamport_ts);
    // The Host's one ingest (plan E3.1 F3): it opens the body, runs the D26
    // gate (block list, bans, timeouts, slowmode, automod), stores the row
    // and publishes the post and the channel's unread count.
    let ingested = state
        .host
        .get()
        .await
        .channel()
        .ingest(ReceivedPost {
            community_id: pending.community_id.clone(),
            channel_id: pending.channel_id.clone(),
            record_key: fetched.record_key.clone(),
            subkey_index: pending.subkey_index,
            sequence: pending.sequence,
            message: message.clone(),
            forwarded_from_author: fetched.forwarded_from_author.clone(),
        })
        .await
        .map_err(|e| e.to_string())?;
    update_peer_sequence(
        state,
        &pending.community_id,
        &message.sender_pseudonym,
        &pending.channel_id,
        pending.sequence,
    );
    let (body, blurred) = match ingested {
        Ingested::Stored { body, blurred } => (body, blurred),
        Ingested::Undecryptable => {
            request_text_key(state, &pending, message.mek_generation);
            emit_message_received(
                state,
                &pending,
                message.sender_pseudonym.clone(),
                String::new(),
                message.timestamp,
                true,
                false,
            );
            return Ok(());
        }
        Ingested::Duplicate | Ingested::Dropped(_) => return Ok(()),
    };
    let sender_pseudonym = message.sender_pseudonym.clone();
    emit_message_received(
        state,
        &pending,
        sender_pseudonym.clone(),
        body.clone(),
        message.timestamp,
        false,
        blurred,
    );
    // An `alert_moderators` match is the Host ingest's `ModReport` and
    // its `AutoModAlert`, which reaches the windows through the journal.

    // Architecture §28.5 line 3120 — pre-decryption notification
    // routing uses the cleartext mention metadata the sender stamped
    // on the envelope. Body parsing is now reserved for tests / legacy
    // payloads that never carried these fields.
    if crate::services::community::should_emit_message_notification(
        state,
        &pool,
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
            &pool,
            &pending.community_id,
            &pending.channel_id,
            &sender_pseudonym,
            &body,
        )
        .await;
    }

    Ok(())
}

/// Ask a peer for the text key of `generation`, which the post needs (A3/
/// P1.3: a retry loop with cascade fall-through, so an offline responder
/// does not leave the message undecryptable until the next rotation).
fn request_text_key(state: &Arc<AppState>, pending: &PendingMessageFetch, generation: u64) {
    let requester_pseudonym = {
        let communities = state.communities.read();
        communities
            .get(&pending.community_id)
            .and_then(|community| community.my_pseudonym_key.clone())
    };
    let Some(requester_pseudonym) = requester_pseudonym else {
        return;
    };
    if let Some(scope) =
        state_helpers::text_scope(state, &pending.community_id, &pending.channel_id)
    {
        crate::services::community::spawn_mek_request_with_retry(
            state.clone(),
            pending.community_id.clone(),
            scope,
            generation,
            requester_pseudonym,
        );
    }
}

#[cfg(test)]
#[path = "message_notifications_tests.rs"]
mod tests;
