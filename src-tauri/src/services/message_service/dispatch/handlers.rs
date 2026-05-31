//! Persist + emit handlers for `DirectMessage` and `ChannelMessage`
//! payload variants. Each persists a row via `message_repo`, bumps the
//! unread counter when appropriate, and fires a journaled `ChatEvent`
//! so a hard-quit client can replay through `event_resume`.

use std::sync::Arc;

use crate::channels::ChatEvent;
use crate::db::DbPool;
use crate::db_helpers::db_fire;
use crate::state::AppState;
use crate::state_helpers;

/// Store a direct message in `SQLite` and emit `ChatEvent` to frontend.
pub(super) fn handle_direct_message(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    sender_hex: &str,
    body: &str,
    timestamp: i64,
) {
    // Store in SQLite (scoped to current identity)
    let owner_key = state_helpers::owner_key_or_default(state);
    let sender = sender_hex.to_string();
    let body_clone = body.to_string();
    db_fire(pool, "persist incoming message", move |conn| {
        crate::message_repo::insert_dm(
            conn,
            &owner_key,
            &sender,
            &sender,
            &body_clone,
            timestamp,
            false,
        )
    });

    // Update unread count
    {
        let mut friends = state.friends.write();
        if let Some(friend) = friends.get_mut(sender_hex) {
            friend.unread_count += 1;
        }
    }

    // Emit to frontend
    let event = ChatEvent::MessageReceived {
        from: sender_hex.to_string(),
        body: body.to_string(),
        decryption_failed: false,
        automod_blurred: false,
        timestamp: timestamp.cast_unsigned(),
        conversation_id: sender_hex.to_string(),
        server_message_id: None, // DMs have no message ID
        reply_to_id: None,
        sender_display_name: None, // DMs use friend list for name resolution
    };
    // Phase 10 — journal + emit so a hard-quit mid-stream client can
    // resume from the last cursor it saw and have this DM replayed.
    crate::event_dispatch::emit_journaled(app_handle, state, "chat-event", &event);
}

/// Store a channel message in `SQLite` and emit `ChatEvent` to frontend.
pub(super) fn handle_channel_message(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    sender_hex: &str,
    channel_id: &str,
    body: &str,
    timestamp: i64,
) {
    let owner_key = state_helpers::owner_key_or_default(state);
    let sender = sender_hex.to_string();
    let ch_id = channel_id.to_string();
    let body_clone = body.to_string();
    db_fire(pool, "persist channel message", move |conn| {
        crate::message_repo::insert_channel_message(
            conn,
            &owner_key,
            &ch_id,
            &sender,
            &body_clone,
            timestamp,
            false,
            None,
        )
    });

    let event = ChatEvent::MessageReceived {
        from: sender_hex.to_string(),
        body: body.to_string(),
        decryption_failed: false,
        automod_blurred: false,
        timestamp: timestamp.cast_unsigned(),
        conversation_id: channel_id.to_string(),
        server_message_id: None, // P2P channel messages — ID assigned by sender
        reply_to_id: None,
        sender_display_name: None, // 1:1 channels use friend list for name resolution
    };
    crate::event_dispatch::emit_journaled(app_handle, state, "chat-event", &event);
}
