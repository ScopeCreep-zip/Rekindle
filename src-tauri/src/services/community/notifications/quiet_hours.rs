//! Phase 23.D.3 — global DND + quiet-hours setter/reader extracted
//! from the original flat `notifications.rs`. Quiet-hours math uses
//! `chrono-tz` so DST transitions are handled automatically.

use std::sync::Arc;

use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::state::AppState;
use crate::state_helpers;

use super::QuietHoursSettings;

pub async fn set_quiet_hours(
    state: &Arc<AppState>,
    pool: &DbPool,
    settings: QuietHoursSettings,
) -> Result<(), String> {
    if settings.start_hour > 23 || settings.end_hour > 23 {
        return Err("quiet hours must use 0-23 hours".to_string());
    }
    // Architecture §17.2 — IANA name is the only authoritative source
    // for the local-time computation. Reject unknown zones at write
    // time so the resolver never sees a value it can't parse.
    if settings.timezone.parse::<chrono_tz::Tz>().is_err() {
        return Err(format!("unknown timezone: {}", settings.timezone));
    }

    let owner_key = state_helpers::current_owner_key(state)?;
    db_call(pool, move |conn| {
        conn.execute(
            "INSERT INTO app_settings
             (owner_key, quiet_hours_enabled, quiet_hours_start_minute, quiet_hours_end_minute, quiet_hours_timezone)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(owner_key)
             DO UPDATE SET
               quiet_hours_enabled = excluded.quiet_hours_enabled,
               quiet_hours_start_minute = excluded.quiet_hours_start_minute,
               quiet_hours_end_minute = excluded.quiet_hours_end_minute,
               quiet_hours_timezone = excluded.quiet_hours_timezone",
            rusqlite::params![
                owner_key,
                i32::from(settings.enabled),
                settings.start_minute(),
                settings.end_minute(),
                settings.timezone,
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
            "SELECT quiet_hours_enabled, quiet_hours_start_minute, quiet_hours_end_minute, quiet_hours_timezone
             FROM app_settings WHERE owner_key = ?1",
        )?;
        let mut rows = stmt.query(rusqlite::params![owner_key])?;
        if let Some(row) = rows.next()? {
            Ok(QuietHoursSettings::from_db(
                row.get::<_, i64>(0).unwrap_or(0),
                row.get::<_, i64>(1).unwrap_or(1320),
                row.get::<_, i64>(2).unwrap_or(420),
                row.get::<_, String>(3).unwrap_or_else(|_| "UTC".to_string()),
            ))
        } else {
            Ok(QuietHoursSettings::default())
        }
    })
    .await
}

/// Read the global Do Not Disturb flag for the current user. Falls
/// back to `false` when unset or on any DB error so a malfunctioning
/// settings table can never silently swallow notifications.
pub async fn is_do_not_disturb_active(state: &Arc<AppState>, pool: &DbPool) -> bool {
    let Ok(owner_key) = state_helpers::current_owner_key(state) else {
        return false;
    };
    crate::db_helpers::db_call_or_default(pool, move |conn| {
        conn.query_row(
            "SELECT do_not_disturb FROM app_settings WHERE owner_key = ?1",
            rusqlite::params![owner_key],
            |r| r.get::<_, i64>(0).map(|v| v != 0),
        )
        .or(Ok(false))
    })
    .await
}

/// Toggle the global Do Not Disturb flag for the current user.
pub async fn set_do_not_disturb(
    state: &Arc<AppState>,
    pool: &DbPool,
    enabled: bool,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state)?;
    let value = i64::from(enabled);
    db_call(pool, move |conn| {
        conn.execute(
            "INSERT INTO app_settings (owner_key, do_not_disturb) \
             VALUES (?1, ?2) \
             ON CONFLICT(owner_key) DO UPDATE SET do_not_disturb = excluded.do_not_disturb",
            rusqlite::params![owner_key, value],
        )?;
        Ok(())
    })
    .await
}

pub(super) async fn is_quiet_hours_active(state: &Arc<AppState>, pool: &DbPool) -> Result<bool, String> {
    use chrono::{Timelike, Utc};

    let settings = get_quiet_hours(state, pool).await?;
    if !settings.enabled {
        return Ok(false);
    }

    // Architecture §17.2 — IANA zone drives the local-time computation
    // via `chrono-tz`. `with_timezone` produces the unambiguous wall
    // clock for the current UTC instant in the configured zone (UTC→local
    // conversions are never ambiguous; only local→UTC is, which we
    // never do here).
    let tz: chrono_tz::Tz = settings
        .timezone
        .parse()
        .map_err(|e| format!("invalid timezone {}: {e}", settings.timezone))?;
    let local = Utc::now().with_timezone(&tz);
    let local_minutes = i64::from(local.hour()) * 60 + i64::from(local.minute());
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
