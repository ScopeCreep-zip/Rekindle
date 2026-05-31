//! Phase 23.D.3 — should-emit gate (DND + quiet hours + mention
//! rules) and `emit_message_notification` throttle/burst-summary
//! fan-out extracted from the original flat `notifications.rs`.

use std::sync::Arc;

use crate::channels::NotificationEvent;
use crate::db::DbPool;
use crate::state::AppState;
use crate::state_helpers;

use super::level::resolve_notification_level;
use super::quiet_hours::{is_do_not_disturb_active, is_quiet_hours_active};
use super::sound::resolve_notification_sound;
use super::{CleartextMentions, NotificationDecision, NotificationLevel};

pub async fn should_emit_message_notification(
    state: &Arc<AppState>,
    pool: &DbPool,
    community_id: &str,
    channel_id: &str,
    sender_pseudonym_hex: &str,
    cleartext: CleartextMentions<'_>,
) -> Result<bool, String> {
    use rekindle_types::channel::flags::{MENTION_EVERYONE, MENTION_HERE, SUPPRESS_NOTIFICATIONS};

    // Architecture §32 Phase 7 Week 25 — Do Not Disturb suppresses
    // every notification regardless of the channel level, mention
    // status, or quiet-hours window. Checked first so the rest of the
    // resolution work is skipped under DND.
    if is_do_not_disturb_active(state, pool).await {
        return Ok(false);
    }
    // SUPPRESS_NOTIFICATIONS is per-message (sender opted-out of
    // pinging anyone) — also takes precedence over mentions.
    if cleartext.flags & SUPPRESS_NOTIFICATIONS != 0 {
        return Ok(false);
    }
    if is_quiet_hours_active(state, pool).await? {
        return Ok(false);
    }

    let level = resolve_notification_level(state, community_id, channel_id);

    // Architecture §28.5 + §9.3 (reader-validates): rebuild a
    // `MentionMatches` from the cleartext envelope fields, then strip
    // privileged classes (@everyone/@here) when the sender lacks
    // `MENTION_EVERYONE`. Body text is preserved as written; only the
    // *escalation* effect is gated.
    let mention_everyone = cleartext.flags & MENTION_EVERYONE != 0;
    let mention_here = cleartext.flags & MENTION_HERE != 0;
    let mut mentions = super::super::mentions::matches_from_cleartext(
        state,
        community_id,
        cleartext.mentioned_pseudonyms,
        cleartext.mentioned_roles,
        mention_everyone,
        mention_here,
    );
    super::super::mentions::validate_sender_permissions(
        state,
        community_id,
        sender_pseudonym_hex,
        &mut mentions,
    );
    let mentioned = super::super::mentions::local_member_is_mentioned(state, community_id, &mentions);

    Ok(match level {
        NotificationLevel::All => true,
        NotificationLevel::Mentions | NotificationLevel::Nothing => mentioned,
    })
}

pub async fn emit_message_notification(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    community_id: &str,
    channel_id: &str,
    sender_pseudonym: &str,
    body: &str,
) {
    let decision = state
        .notification_throttle
        .record_attempt_now(community_id, channel_id);
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
        resolve_notification_sound(pool, &owner_key, community_id, channel_id).await
    };

    let (title, payload_body) = match decision {
        NotificationDecision::EmitSummary { bundled_count } => {
            let title = format!("#{channel_name}");
            let body = format!(
                "[{community_name}] {bundled_count} more messages in #{channel_name}"
            );
            (title, body)
        }
        NotificationDecision::Emit => {
            let sender_name = sender_pseudonym.chars().take(8).collect::<String>();
            let title = format!("{sender_name} in #{channel_name}");
            let body = format!("[{community_name}] {body}");
            (title, body)
        }
        NotificationDecision::Drop => unreachable!("Drop returned early above"),
    };

    crate::event_dispatch::emit_live(
        app_handle,
        "notification-event",
        &NotificationEvent::MessageReceived {
            title,
            body: payload_body,
            community_id: community_id.to_string(),
            channel_id: channel_id.to_string(),
            sound_ref,
        },
    );
}
