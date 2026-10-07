//! Persist + emit handlers for `DirectMessage` and `ChannelMessage`
//! payload variants. Each persists a row via `message_repo`, bumps the
//! unread counter when appropriate, and fires a journaled event so a
//! window that reloads mid-stream can recover it (`subscribe_events`).

use std::sync::Arc;

use crate::channels::ChatEvent;
use crate::db_helpers::db_fire;
use crate::state::AppState;
use crate::state_helpers;
use rekindle_db::Db;

/// Store a direct message in `SQLite` and emit `ChatEvent` to frontend.
pub(super) fn handle_direct_message(
    state: &Arc<AppState>,
    pool: &Db,
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
    let event = rekindle_types::subscription_events::SubscriptionEvent::ChannelMessage(
        rekindle_types::subscription_events::ChannelMessageEvent::DirectMessageReceived {
            peer_key: sender_hex.to_string(),
            body: Some(body.to_string()),
            decryption_failed: false,
            automod_blurred: false,
            timestamp: timestamp.cast_unsigned(),
            conversation_id: sender_hex.to_string(),
            server_message_id: None, // DMs have no message ID
            reply_to_id: None,
            sender_name: None, // DMs use friend list for name resolution
        },
    );
    // Journaled so a chat window that reloads mid-stream gets it.
    crate::event_dispatch::emit_journaled(
        state,
        crate::event_dispatch::WebviewEvent::Subscription(event),
    );
}

/// Store a channel message in `SQLite` and emit `ChatEvent` to frontend.
pub(super) fn handle_channel_message(
    state: &Arc<AppState>,
    pool: &Db,
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
    // A channel message over a 1:1 envelope names no community, so it can
    // only go to every community window (plan N7 decides this arm's fate).
    crate::event_dispatch::emit_journaled(
        state,
        crate::event_dispatch::WebviewEvent::ChannelChat {
            community_id: None,
            event,
        },
    );
}
