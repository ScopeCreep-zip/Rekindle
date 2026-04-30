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
pub async fn set_quiet_hours(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    enabled: bool,
    start_hour: u8,
    end_hour: u8,
    utc_offset_minutes: Option<i16>,
) -> Result<(), String> {
    crate::services::community::notifications::set_quiet_hours(
        state.inner(),
        pool.inner(),
        crate::services::community::notifications::QuietHoursSettings {
            enabled,
            start_hour,
            end_hour,
            utc_offset_minutes: utc_offset_minutes.unwrap_or(0),
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
        utc_offset_minutes: settings.utc_offset_minutes,
    })
}
