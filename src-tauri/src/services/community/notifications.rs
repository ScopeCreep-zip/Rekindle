use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::Emitter;

use crate::channels::NotificationEvent;
use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::state::AppState;
use crate::state_helpers;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationLevel {
    All,
    Mentions,
    Nothing,
}

impl NotificationLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Mentions => "mentions",
            Self::Nothing => "nothing",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "all" => Some(Self::All),
            "mentions" => Some(Self::Mentions),
            "nothing" => Some(Self::Nothing),
            _ => None,
        }
    }

    fn to_db(self) -> i64 {
        match self {
            Self::All => 0,
            Self::Mentions => 1,
            Self::Nothing => 2,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct QuietHoursSettings {
    pub enabled: bool,
    pub start_hour: u8,
    pub end_hour: u8,
    pub utc_offset_minutes: i16,
}

impl Default for QuietHoursSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            start_hour: 22,
            end_hour: 7,
            utc_offset_minutes: 0,
        }
    }
}

impl QuietHoursSettings {
    fn from_db(enabled: i64, start_minute: i64, end_minute: i64, utc_offset_minutes: i64) -> Self {
        Self {
            enabled: enabled != 0,
            start_hour: u8::try_from(start_minute.div_euclid(60)).unwrap_or(22),
            end_hour: u8::try_from(end_minute.div_euclid(60)).unwrap_or(7),
            utc_offset_minutes: i16::try_from(utc_offset_minutes).unwrap_or(0),
        }
    }

    fn start_minute(self) -> i64 {
        i64::from(self.start_hour) * 60
    }

    fn end_minute(self) -> i64 {
        i64::from(self.end_hour) * 60
    }
}

pub fn parse_notification_level(level: &str) -> Result<NotificationLevel, String> {
    NotificationLevel::from_str(level)
        .ok_or_else(|| "notification level must be one of: all, mentions, nothing".to_string())
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

pub async fn set_quiet_hours(
    state: &Arc<AppState>,
    pool: &DbPool,
    settings: QuietHoursSettings,
) -> Result<(), String> {
    if settings.start_hour > 23 || settings.end_hour > 23 {
        return Err("quiet hours must use 0-23 hours".to_string());
    }

    let owner_key = state_helpers::current_owner_key(state)?;
    db_call(pool, move |conn| {
        conn.execute(
            "INSERT INTO app_settings
             (owner_key, quiet_hours_enabled, quiet_hours_start_minute, quiet_hours_end_minute, quiet_hours_utc_offset_minutes)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(owner_key)
             DO UPDATE SET
               quiet_hours_enabled = excluded.quiet_hours_enabled,
               quiet_hours_start_minute = excluded.quiet_hours_start_minute,
               quiet_hours_end_minute = excluded.quiet_hours_end_minute,
               quiet_hours_utc_offset_minutes = excluded.quiet_hours_utc_offset_minutes",
            rusqlite::params![
                owner_key,
                i32::from(settings.enabled),
                settings.start_minute(),
                settings.end_minute(),
                i32::from(settings.utc_offset_minutes),
            ],
        )?;
        Ok(())
    })
    .await
}

pub async fn get_quiet_hours(
    state: &Arc<AppState>,
    pool: &DbPool,
) -> Result<QuietHoursSettings, String> {
    let owner_key = state_helpers::current_owner_key(state)?;
    db_call(pool, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT quiet_hours_enabled, quiet_hours_start_minute, quiet_hours_end_minute, quiet_hours_utc_offset_minutes
             FROM app_settings WHERE owner_key = ?1",
        )?;
        let mut rows = stmt.query(rusqlite::params![owner_key])?;
        if let Some(row) = rows.next()? {
            Ok(QuietHoursSettings::from_db(
                row.get::<_, i64>(0).unwrap_or(0),
                row.get::<_, i64>(1).unwrap_or(1320),
                row.get::<_, i64>(2).unwrap_or(420),
                row.get::<_, i64>(3).unwrap_or(0),
            ))
        } else {
            Ok(QuietHoursSettings::default())
        }
    })
    .await
}

pub async fn should_emit_message_notification(
    state: &Arc<AppState>,
    pool: &DbPool,
    community_id: &str,
    channel_id: &str,
    body: &str,
) -> Result<bool, String> {
    if is_quiet_hours_active(state, pool).await? {
        return Ok(false);
    }

    let level = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .and_then(|community| {
                community
                    .channels
                    .iter()
                    .find(|channel| channel.id == channel_id)
                    .map(|channel| channel.notification_level.as_str())
            })
            .and_then(NotificationLevel::from_str)
            .unwrap_or(NotificationLevel::All)
    };

    Ok(match level {
        NotificationLevel::All => true,
        NotificationLevel::Mentions => body_mentions_local_member(state, community_id, body),
        NotificationLevel::Nothing => false,
    })
}

pub fn emit_message_notification(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    sender_pseudonym: &str,
    body: &str,
) {
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

    let sender_name = sender_pseudonym.chars().take(8).collect::<String>();
    let title = format!("{sender_name} in #{channel_name}");
    let body = format!("[{community_name}] {body}");
    let _ = app_handle.emit("notification-event", &NotificationEvent::SystemAlert { title, body });
}

async fn is_quiet_hours_active(state: &Arc<AppState>, pool: &DbPool) -> Result<bool, String> {
    let settings = get_quiet_hours(state, pool).await?;
    if !settings.enabled {
        return Ok(false);
    }

    let utc_now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs();
    let utc_minutes = i64::try_from((utc_now % 86_400) / 60).unwrap_or(0);
    let local_minutes =
        (utc_minutes + i64::from(settings.utc_offset_minutes)).rem_euclid(24 * 60);
    let start_minute = settings.start_minute();
    let end_minute = settings.end_minute();

    Ok(match start_minute.cmp(&end_minute) {
        std::cmp::Ordering::Equal => true,
        std::cmp::Ordering::Less => {
            local_minutes >= start_minute && local_minutes < end_minute
        }
        std::cmp::Ordering::Greater => {
            local_minutes >= start_minute || local_minutes < end_minute
        }
    })
}

fn body_mentions_local_member(state: &Arc<AppState>, community_id: &str, body: &str) -> bool {
    let lower_body = body.to_lowercase();
    if lower_body.contains("@everyone") || lower_body.contains("@here") || lower_body.contains("@me")
    {
        return true;
    }

    let communities = state.communities.read();
    let Some(community) = communities.get(community_id) else {
        return false;
    };

    community
        .my_role_ids
        .iter()
        .filter_map(|role_id| community.roles.iter().find(|role| role.id == *role_id))
        .map(|role| format!("@{}", role.name.to_lowercase()))
        .any(|mention| lower_body.contains(&mention))
}
