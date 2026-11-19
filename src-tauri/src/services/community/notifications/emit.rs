//! Phase 23.D.3 — should-emit gate (DND + quiet hours + mention
//! rules) and `emit_message_notification` throttle/burst-summary
//! fan-out extracted from the original flat `notifications.rs`;
//! `notify_channel_post` runs both for a post the Host stored.

use std::sync::Arc;

use rekindle_types::subscription_events::{ChannelMessageEvent, SubscriptionEvent};

use crate::state::AppState;
use crate::state_helpers;
use rekindle_db::Db;

use super::level::resolve_notification_level;
use super::quiet_hours::{is_do_not_disturb_active, is_quiet_hours_active};
use super::sound::resolve_notification_sound;
use super::{NotificationDecision, NotificationLevel};

/// Whether a received post notifies: DND, the sender's
/// `SUPPRESS_NOTIFICATIONS`, quiet hours, then the channel's level.
/// `mentioned` is the Host ingest's reader-validated verdict
/// (`messages.mentions_me`, architecture §28.5 + §9.3: @everyone and @here
/// count only when the sender holds `MENTION_EVERYONE`).
async fn should_emit_message_notification(
    state: &Arc<AppState>,
    pool: &Db,
    community_id: &str,
    channel_id: &str,
    mentioned: bool,
    flags: u32,
) -> Result<bool, String> {
    use rekindle_types::channel::flags::SUPPRESS_NOTIFICATIONS;

    // Architecture §32 Phase 7 Week 25 — Do Not Disturb suppresses
    // every notification regardless of the channel level, mention
    // status, or quiet-hours window. Checked first so the rest of the
    // resolution work is skipped under DND.
    if is_do_not_disturb_active(state, pool).await {
        return Ok(false);
    }
    // SUPPRESS_NOTIFICATIONS is per-message (sender opted-out of
    // pinging anyone) — also takes precedence over mentions.
    if flags & SUPPRESS_NOTIFICATIONS != 0 {
        return Ok(false);
    }
    if is_quiet_hours_active(state, pool).await? {
        return Ok(false);
    }

    let level = resolve_notification_level(state, community_id, channel_id);

    Ok(match level {
        NotificationLevel::All => true,
        NotificationLevel::Mentions | NotificationLevel::Nothing => mentioned,
    })
}

async fn emit_message_notification(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &Db,
    community_id: &str,
    channel_id: &str,
    sender_pseudonym: &str,
    body: &str,
) {
    let decision = state.notification_throttle.record_attempt(
        community_id,
        channel_id,
        u128::from(state.clock.now_ms()),
    );
    if matches!(decision, NotificationDecision::Drop) {
        return;
    }

    let (community_name, channel_name) = {
        let communities = state.communities.read();
        let community = communities.get(community_id);
        let community_name = community.map_or_else(
            || "Community".to_string(),
            |community| community.name.clone(),
        );
        let channel_name = community
            .and_then(|community| {
                community
                    .channels
                    .iter()
                    .find(|channel| channel.id == channel_id)
                    .map(|channel| channel.name.clone())
            })
            .unwrap_or_else(|| "channel".to_string());
        (community_name, channel_name)
    };

    let owner_key = state_helpers::owner_key_or_default(state);
    let sound_ref = if owner_key.is_empty() {
        None
    } else {
        resolve_notification_sound(pool, &owner_key, community_id, channel_id)
            .await
            .unwrap_or_else(|e| {
                // No override readable: the app-global sound toggle applies.
                tracing::warn!(error = %e, "notification sound override unreadable");
                None
            })
    };

    let (title, payload_body) = match decision {
        NotificationDecision::EmitSummary { bundled_count } => {
            let title = format!("#{channel_name}");
            let body =
                format!("[{community_name}] {bundled_count} more messages in #{channel_name}");
            (title, body)
        }
        NotificationDecision::Emit => {
            let sender_name = crate::state_helpers::peer_label(state, sender_pseudonym);
            let title = format!("{sender_name} in #{channel_name}");
            let body = format!("[{community_name}] {body}");
            (title, body)
        }
        NotificationDecision::Drop => unreachable!("Drop returned early above"),
    };

    crate::event_dispatch::emit_notification(
        app_handle,
        rekindle_types::subscription_events::NotificationEvent::MessageReceived {
            title,
            body: payload_body,
            community_id: community_id.to_string(),
            channel_id: channel_id.to_string(),
            sound_ref,
        },
    );
}

/// Notify a post another member wrote, which the Host stored and
/// published as `ChannelMessageEvent::New` (every read path: the outbox
/// read and the slot reads), through the should-emit gate and the
/// throttle. The gate's inputs are the stored row's.
async fn notify_channel_post(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    post: ChannelPostNotice,
) {
    let Ok(pool) = state.db.current() else {
        return;
    };
    let owner_key = state_helpers::owner_key_or_default(state);
    if owner_key.is_empty() {
        return;
    }
    let message_id = post.message_id.clone();
    let fields = crate::db_helpers::db_call(&pool, move |conn| {
        rekindle_db::repo::messages::channel::notify_fields(conn, &owner_key, &message_id)
    })
    .await;
    let (mentioned, flags) = match fields {
        Ok(Some(fields)) => fields,
        Ok(None) => return,
        Err(error) => {
            tracing::warn!(%error, "channel post notification fields unreadable");
            return;
        }
    };
    let emit = should_emit_message_notification(
        state,
        &pool,
        &post.community_id,
        &post.channel_id,
        mentioned,
        flags,
    )
    .await
    .unwrap_or(false);
    if emit {
        emit_message_notification(
            app_handle,
            state,
            &pool,
            &post.community_id,
            &post.channel_id,
            &post.sender_pseudonym,
            &post.body,
        )
        .await;
    }
}

/// A stored post `notify_channel_post` notifies.
struct ChannelPostNotice {
    community_id: String,
    channel_id: String,
    message_id: String,
    sender_pseudonym: String,
    body: String,
}

/// A Host event the desktop shell receives: a `ChannelMessageEvent::New`
/// for another member's shown post is notified on the login scope.
pub fn notify_stored_post(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    event: &SubscriptionEvent,
) {
    let SubscriptionEvent::ChannelMessage(ChannelMessageEvent::New {
        community,
        channel,
        message_id,
        sender_pseudonym,
        body: Some(body),
        automod_blurred: false,
        ..
    }) = event
    else {
        return;
    };
    let own = state
        .communities
        .read()
        .get(community)
        .and_then(|c| c.my_pseudonym_key.clone())
        .is_some_and(|me| me.eq_ignore_ascii_case(sender_pseudonym));
    if own {
        return;
    }
    let post = ChannelPostNotice {
        community_id: community.clone(),
        channel_id: channel.clone(),
        message_id: message_id.clone(),
        sender_pseudonym: sender_pseudonym.clone(),
        body: body.clone(),
    };
    let (app, state_owned) = (app_handle.clone(), Arc::clone(state));
    state_helpers::spawn_in_login_with_token(
        state,
        "channel post notification",
        move |stop| async move {
            stop.run_until_cancelled(notify_channel_post(&app, &state_owned, post))
                .await;
        },
    );
}
