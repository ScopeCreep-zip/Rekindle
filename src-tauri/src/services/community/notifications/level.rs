//! Phase 23.D.3 — `NotificationLevel` parser + tier resolver +
//! community-default / per-channel mutators extracted from the
//! original flat `notifications.rs`.

use std::sync::Arc;

use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::state::AppState;
use crate::state_helpers;

use super::NotificationLevel;

pub use rekindle_channel::parse_notification_level;

/// Architecture §17.1 three-tier cascade resolver. Most-specific wins:
///
/// 1. **Per-channel override** — local-only, stored on `ChannelInfo.notification_level`.
/// 2. **Community default** — governance-broadcast `CommunityNotificationDefault`.
/// 3. **Implicit "all"** — when neither tier 1 nor tier 2 is set.
///
/// User-level DND is layered on top by `should_emit_message_notification`
/// (the quiet-hours short-circuit) and so isn't part of this tier
/// resolution.
pub fn resolve_notification_level(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
) -> NotificationLevel {
    let communities = state.communities.read();
    let Some(community) = communities.get(community_id) else {
        return NotificationLevel::All;
    };

    // Tier 1: per-channel override.
    let channel_level = community
        .channels
        .iter()
        .find(|channel| channel.id == channel_id)
        .and_then(|channel| channel.notification_level.parse::<NotificationLevel>().ok());
    if let Some(level) = channel_level {
        return level;
    }

    // Tier 2: community default.
    if let Some(level) = community
        .governance_state
        .as_ref()
        .and_then(|gov| gov.notification_default.as_ref())
        .and_then(|default| default.level.parse::<NotificationLevel>().ok())
    {
        return level;
    }

    // Tier 3: implicit.
    NotificationLevel::All
}

/// Architecture §17.1 tier 1: write a `CommunityNotificationDefault`
/// governance entry so every member learns the community-wide
/// notification default. Per-channel overrides remain local-only and
/// continue to win in `resolve_notification_level`.
pub async fn set_community_default_notification_level(
    state: &Arc<AppState>,
    community_id: &str,
    level: NotificationLevel,
) -> Result<(), String> {
    let lamport = state_helpers::increment_lamport(state, community_id);
    super::super::governance::write_entry(
        state,
        community_id,
        rekindle_types::governance::GovernanceEntry::CommunityNotificationDefault {
            level: level.as_str().to_string(),
            lamport,
        },
    )
    .await
}

pub fn get_community_default_notification_level(
    state: &Arc<AppState>,
    community_id: &str,
) -> Option<NotificationLevel> {
    let communities = state.communities.read();
    communities
        .get(community_id)
        .and_then(|c| c.governance_state.as_ref())
        .and_then(|gov| gov.notification_default.as_ref())
        .and_then(|d| d.level.parse::<NotificationLevel>().ok())
}

pub async fn set_channel_notification_level(
    state: &Arc<AppState>,
    pool: &DbPool,
    community_id: &str,
    channel_id: &str,
    level: NotificationLevel,
) -> Result<(), String> {
    {
        let mut communities = state.communities.write();
        let community = communities
            .get_mut(community_id)
            .ok_or("community not found")?;
        let channel = community
            .channels
            .iter_mut()
            .find(|channel| channel.id == channel_id)
            .ok_or("channel not found")?;
        channel.notification_level = level.as_str().to_string();
    }

    let owner_key = state_helpers::current_owner_key(state)?;
    let community_id_owned = community_id.to_string();
    let channel_id_owned = channel_id.to_string();
    db_call(pool, move |conn| {
        conn.execute(
            "INSERT INTO notification_preferences (owner_key, community_id, channel_id, level)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(owner_key, community_id, channel_id)
             DO UPDATE SET level = excluded.level",
            rusqlite::params![
                owner_key,
                community_id_owned,
                channel_id_owned,
                level.to_db(),
            ],
        )?;
        Ok(())
    })
    .await
}
