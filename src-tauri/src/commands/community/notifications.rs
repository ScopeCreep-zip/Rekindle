use tauri::State;

use crate::commands::community::types::QuietHoursSettingsDto;
use crate::db::DbPool;
use crate::state::SharedState;

#[tauri::command]
pub async fn set_channel_notification_level(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    channel_id: String,
    level: String,
) -> Result<(), String> {
    let parsed = crate::services::community::notifications::parse_notification_level(&level)?;
    crate::services::community::notifications::set_channel_notification_level(
        state.inner(),
        pool.inner(),
        &community_id,
        &channel_id,
        parsed,
    )
    .await
}

#[tauri::command]
pub async fn set_community_default_notification_level(
    state: State<'_, SharedState>,
    community_id: String,
    level: String,
) -> Result<(), String> {
    let parsed = crate::services::community::notifications::parse_notification_level(&level)?;
    crate::services::community::notifications::set_community_default_notification_level(
        state.inner(),
        &community_id,
        parsed,
    )
    .await
}

#[tauri::command]
pub async fn get_community_default_notification_level(
    state: State<'_, SharedState>,
    community_id: String,
) -> Result<Option<String>, String> {
    Ok(
        crate::services::community::notifications::get_community_default_notification_level(
            state.inner(),
            &community_id,
        )
        .map(|level| level.as_str().to_string()),
    )
}

/// Architecture §32 Phase 7 Week 25 — set the notification sound for
/// a channel (`channel_id` non-empty) or for the community default
/// (`channel_id = ""`). `sound_ref = None` removes the override.
#[tauri::command]
pub async fn set_notification_sound(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    channel_id: String,
    sound_ref: Option<String>,
) -> Result<(), String> {
    crate::services::community::notifications::set_notification_sound(
        state.inner(),
        pool.inner(),
        &community_id,
        &channel_id,
        sound_ref,
    )
    .await
}

/// Resolve the effective notification sound for `(community, channel)`
/// using the channel override → community default → `None` fallthrough.
/// Frontend uses the result to play the sound (or fall back to its
/// own bundled default if `None`).
#[tauri::command]
pub async fn get_notification_sound(
    pool: State<'_, DbPool>,
    state: State<'_, SharedState>,
    community_id: String,
    channel_id: String,
) -> Result<Option<String>, String> {
    let owner_key = crate::state_helpers::current_owner_key(state.inner())?;
    Ok(crate::services::community::notifications::resolve_notification_sound(
        pool.inner(),
        &owner_key,
        &community_id,
        &channel_id,
    )
    .await)
}

/// Architecture §32 Phase 7 Week 25 — Do Not Disturb global toggle.
/// When `true`, every notification dispatch is suppressed regardless
/// of channel level, mention status, or quiet-hours window.
#[tauri::command]
pub async fn set_do_not_disturb(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    enabled: bool,
) -> Result<(), String> {
    crate::services::community::notifications::set_do_not_disturb(
        state.inner(),
        pool.inner(),
        enabled,
    )
    .await
}

#[tauri::command]
pub async fn get_do_not_disturb(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<bool, String> {
    Ok(crate::services::community::notifications::is_do_not_disturb_active(
        state.inner(),
        pool.inner(),
    )
    .await)
}

#[tauri::command]
pub async fn set_quiet_hours(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    enabled: bool,
    start_hour: u8,
    end_hour: u8,
    timezone: String,
) -> Result<(), String> {
    crate::services::community::notifications::set_quiet_hours(
        state.inner(),
        pool.inner(),
        crate::services::community::notifications::QuietHoursSettings {
            enabled,
            start_hour,
            end_hour,
            timezone,
        },
    )
    .await
}

#[tauri::command]
pub async fn get_quiet_hours(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<QuietHoursSettingsDto, String> {
    let settings =
        crate::services::community::notifications::get_quiet_hours(state.inner(), pool.inner())
            .await?;
    Ok(QuietHoursSettingsDto {
        enabled: settings.enabled,
        start_hour: settings.start_hour,
        end_hour: settings.end_hour,
        timezone: settings.timezone,
    })
}
