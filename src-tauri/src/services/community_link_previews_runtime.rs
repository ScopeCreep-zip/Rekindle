//! Phase 23.C — link-preview settings handlers lifted from
//! `commands/community/link_previews.rs`. Hosts the
//! `app_settings.link_previews_enabled` get/set SQLite operations.

use crate::db_helpers::{db_call, db_call_or_default};
use crate::state::SharedState;
use crate::state_helpers;
use rekindle_db::Db;

pub async fn set_link_previews_enabled_inner(
    state: &SharedState,
    pool: &Db,
    enabled: bool,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state)?;
    let value = i64::from(enabled);
    db_call(pool, move |conn| {
        conn.execute(
            "INSERT INTO app_settings (owner_key, link_previews_enabled) VALUES (?1, ?2) \
             ON CONFLICT(owner_key) DO UPDATE SET link_previews_enabled = excluded.link_previews_enabled",
            rusqlite::params![owner_key, value],
        )?;
        Ok(())
    })
    .await
}

pub async fn get_link_previews_enabled_inner(
    state: &SharedState,
    pool: &Db,
) -> Result<bool, String> {
    let owner_key = state_helpers::current_owner_key(state)?;
    Ok(link_previews_enabled(pool, owner_key).await)
}

/// The user's link-preview setting. Previews reveal this device's IP to
/// the linked site, so anything short of an explicit "on" (no settings
/// row, a DB error) means off.
pub async fn link_previews_enabled(pool: &Db, owner_key: String) -> bool {
    db_call_or_default(pool, move |conn| {
        let value: Option<i64> = conn
            .query_row(
                "SELECT link_previews_enabled FROM app_settings WHERE owner_key = ?1",
                rusqlite::params![owner_key],
                |row| row.get(0),
            )
            .ok();
        Ok(value.is_some_and(|v| v != 0))
    })
    .await
}
