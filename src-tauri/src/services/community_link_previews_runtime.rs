//! Phase 23.C — link-preview settings handlers lifted from
//! `commands/community/link_previews.rs`. Hosts the
//! `app_settings.link_previews_enabled` get/set SQLite operations.

use crate::db::DbPool;
use crate::db_helpers::{db_call, db_call_or_default};
use crate::state::SharedState;
use crate::state_helpers;

pub async fn set_link_previews_enabled_inner(
    state: &SharedState,
    pool: &DbPool,
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
    pool: &DbPool,
) -> Result<bool, String> {
    let owner_key = state_helpers::current_owner_key(state)?;
    Ok(db_call_or_default(pool, move |conn| {
        let value: Option<i64> = conn
            .query_row(
                "SELECT link_previews_enabled FROM app_settings WHERE owner_key = ?1",
                rusqlite::params![owner_key],
                |row| row.get(0),
            )
            .ok();
        Ok(value.unwrap_or(1) != 0)
    })
    .await)
}
