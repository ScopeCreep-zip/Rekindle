//! Phase 23.D.4 — `ChannelEvent` → src-tauri event mapping +
//! local-echo + delivery-state emits extracted from `deps_impl.rs`.

use rekindle_channel::deps::SentChannelMessageEcho;
use rekindle_channel::event::ChannelEvent;

use crate::channels::{ChatEvent, CommunityEvent};
use crate::event_dispatch::{emit, WebviewEvent};

use tauri::AppHandle;

use super::ChannelAdapter;

pub(super) fn emit_event_impl(_adapter: &ChannelAdapter, event: ChannelEvent) {
    match event {
        ChannelEvent::MessageSent {
            community_id,
            channel_id,
            message_id,
            sender_pseudonym,
        } => {
            tracing::debug!(community = %community_id, channel = %channel_id, msg = %message_id, sender = %sender_pseudonym, "channel message sent");
        }
        ChannelEvent::MessageReceived {
            community_id,
            channel_id,
            message_id,
            sender_pseudonym,
            mentioned_local,
        } => {
            tracing::debug!(community = %community_id, channel = %channel_id, msg = %message_id, sender = %sender_pseudonym, mentioned = mentioned_local, "channel message received");
        }
        ChannelEvent::ThreadCreated {
            community_id,
            channel_id,
            thread_id,
            parent_message_id,
        } => {
            tracing::debug!(community = %community_id, channel = %channel_id, thread = %thread_id, parent = %parent_message_id, "thread created");
        }
        ChannelEvent::ThreadMessage {
            community_id,
            thread_id,
            message_id,
            sender_pseudonym,
        } => {
            tracing::debug!(community = %community_id, thread = %thread_id, msg = %message_id, sender = %sender_pseudonym, "thread message");
        }
        ChannelEvent::ReactionChanged {
            community_id,
            channel_id,
            message_id,
            reactor_pseudonym,
            emoji,
            added,
        } => {
            tracing::debug!(community = %community_id, channel = %channel_id, msg = %message_id, reactor = %reactor_pseudonym, %emoji, added, "reaction changed");
        }
        ChannelEvent::ExpressionUploaded {
            community_id,
            expression_id,
            kind,
        } => {
            tracing::debug!(community = %community_id, %expression_id, %kind, "expression uploaded");
        }
        ChannelEvent::ExpressionDeleted {
            community_id,
            expression_id,
        } => {
            tracing::debug!(community = %community_id, %expression_id, "expression deleted");
        }
    }
}

pub(super) fn emit_chat_event_local_impl(adapter: &ChannelAdapter, sent: &SentChannelMessageEcho) {
    echo(&adapter.app_handle, sent);
}

pub(super) fn emit_delivery_succeeded_impl(
    adapter: &ChannelAdapter,
    community_id: &str,
    channel_id: &str,
    message_id: &str,
) {
    post_settled(&adapter.app_handle, community_id, channel_id, message_id, true);
}

pub(super) fn emit_delivery_failed_impl(
    adapter: &ChannelAdapter,
    community_id: &str,
    channel_id: &str,
    message_id: &str,
) {
    post_settled(&adapter.app_handle, community_id, channel_id, message_id, false);
}

/// The echo of our own channel post, in the windows' legacy shape (the
/// Host's `ShellHooks::channel_echo`, until E3.1 E2c).
pub fn echo(app: &AppHandle, echo: &SentChannelMessageEcho) {
    let event = ChatEvent::MessageReceived {
        from: echo.sender_pseudonym.clone(),
        body: echo.body.clone(),
        decryption_failed: false,
        automod_blurred: false,
        timestamp: echo.timestamp_ms / 1000,
        conversation_id: echo.channel_id.clone(),
        server_message_id: Some(echo.message_id.clone()),
        reply_to_id: None,
        sender_display_name: None,
    };
    emit(
        app,
        WebviewEvent::ChannelChat {
            community_id: Some(echo.community_id.clone()),
            event,
        },
    );
}

/// Our post settled (`landed`, else failed), in the windows' legacy shape
/// (`ShellHooks::channel_post_settled`, until E3.1 E2c).
pub fn post_settled(
    app: &AppHandle,
    community_id: &str,
    channel_id: &str,
    message_id: &str,
    landed: bool,
) {
    let (community_id, channel_id, message_id) = (
        community_id.to_string(),
        channel_id.to_string(),
        message_id.to_string(),
    );
    let event = if landed {
        CommunityEvent::ChannelMessageDelivered {
            community_id,
            channel_id,
            message_id,
        }
    } else {
        CommunityEvent::ChannelMessageDeliveryFailed {
            community_id,
            channel_id,
            message_id,
        }
    };
    emit(app, WebviewEvent::Community(event));
}
