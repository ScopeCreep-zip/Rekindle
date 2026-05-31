//! Phase 23.C — blocked-user SQLite operations lifted from
//! `commands/friends.rs`. Hosts `is_user_blocked`, `unblock_user_inner`,
//! and `get_blocked_users_inner`.

use serde::{Deserialize, Serialize};

use crate::db::DbPool;
use crate::db_helpers::{db_call, db_call_or_default};
use crate::state::SharedState;
use crate::state_helpers;

/// A blocked user entry returned by `get_blocked_users`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockedUser {
    pub public_key: String,
    pub display_name: String,
    pub blocked_at: i64,
}

pub async fn is_user_blocked(pool: &DbPool, owner_key: &str, public_key: &str) -> bool {
    let ok = owner_key.to_string();
    let pk = public_key.to_string();
    db_call_or_default(pool, move |conn| {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM blocked_users WHERE owner_key = ?1 AND public_key = ?2",
                rusqlite::params![ok, pk],
                |row| row.get(0),
            )
            .unwrap_or(0);
        Ok(count > 0)
    })
    .await
}

pub async fn unblock_user_inner(
    state: &SharedState,
    pool: &DbPool,
    public_key: String,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state)?;
    let pk = public_key.clone();
    let ok = owner_key;
    db_call(pool, move |conn| {
        conn.execute(
            "DELETE FROM blocked_users WHERE owner_key = ?1 AND public_key = ?2",
            rusqlite::params![ok, pk],
        )?;
        Ok(())
    })
    .await?;

    tracing::info!(public_key = %public_key, "user unblocked");
    Ok(())
}

pub async fn get_blocked_users_inner(
    state: &SharedState,
    pool: &DbPool,
) -> Result<Vec<BlockedUser>, String> {
    let owner_key = state_helpers::current_owner_key(state)?;
    db_call(pool, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT public_key, display_name, blocked_at \
             FROM blocked_users WHERE owner_key = ?1 ORDER BY blocked_at DESC",
        )?;
        let rows = stmt.query_map(rusqlite::params![owner_key], |row| {
            Ok(BlockedUser {
                public_key: row.get(0)?,
                display_name: row.get(1)?,
                blocked_at: row.get(2)?,
            })
        })?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    })
    .await
}
