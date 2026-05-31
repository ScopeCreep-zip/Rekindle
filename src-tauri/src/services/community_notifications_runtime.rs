//! Phase 23.C — quiet-hours wrappers lifted from
//! `commands/community/notifications.rs`. Pure struct-shape conversion
//! between the Tauri DTO and the internal `QuietHoursSettings` shape.

use crate::db::DbPool;
use crate::state::SharedState;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuietHoursSettingsDto {
    pub enabled: bool,
    pub start_hour: u8,
    pub end_hour: u8,
    pub timezone: String,
}

pub async fn set_quiet_hours_inner(
    state: &SharedState,
    pool: &DbPool,
    enabled: bool,
    start_hour: u8,
    end_hour: u8,
    timezone: String,
) -> Result<(), String> {
    crate::services::community::notifications::set_quiet_hours(
        state,
        pool,
        crate::services::community::notifications::QuietHoursSettings {
            enabled,
            start_hour,
            end_hour,
            timezone,
        },
    )
    .await
}

pub async fn get_quiet_hours_inner(
    state: &SharedState,
    pool: &DbPool,
) -> Result<QuietHoursSettingsDto, String> {
    let settings = crate::services::community::notifications::get_quiet_hours(state, pool).await?;
    Ok(QuietHoursSettingsDto {
        enabled: settings.enabled,
        start_hour: settings.start_hour,
        end_hour: settings.end_hour,
        timezone: settings.timezone,
    })
}

pub async fn set_channel_notification_level_inner(
    state: &SharedState,
    pool: &DbPool,
    community_id: &str,
    channel_id: &str,
    level: &str,
) -> Result<(), String> {
    let parsed = crate::services::community::notifications::parse_notification_level(level)?;
    crate::services::community::notifications::set_channel_notification_level(
        state,
        pool,
        community_id,
        channel_id,
        parsed,
    )
    .await
}

pub async fn set_community_default_notification_level_inner(
    state: &SharedState,
    community_id: &str,
    level: &str,
) -> Result<(), String> {
    let parsed = crate::services::community::notifications::parse_notification_level(level)?;
    crate::services::community::notifications::set_community_default_notification_level(
        state,
        community_id,
        parsed,
    )
    .await
}

pub fn get_community_default_notification_level_inner(
    state: &SharedState,
    community_id: &str,
) -> Option<String> {
    crate::services::community::notifications::get_community_default_notification_level(
        state,
        community_id,
    )
    .map(|level| level.as_str().to_string())
}

pub async fn get_notification_sound_inner(
    state: &SharedState,
    pool: &DbPool,
    community_id: &str,
    channel_id: &str,
) -> Result<Option<String>, String> {
    let owner_key = crate::state_helpers::current_owner_key(state)?;
    Ok(
        crate::services::community::notifications::resolve_notification_sound(
            pool,
            &owner_key,
            community_id,
            channel_id,
        )
        .await,
    )
}
