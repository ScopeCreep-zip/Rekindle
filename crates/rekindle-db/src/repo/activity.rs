//! `activity_log` (plan E7.15, IV1): the local activity feed the
//! Infoview's Activity section lists. Derived on this device from what
//! friends already sent us and from our own actions
//! (`rekindle_presence::activity`); never synced or sent. It is a history
//! of our friends' behaviour, so it is bounded to [`MAX_ROWS`] rows and
//! [`MAX_AGE_MS`], pruned on every insert, and a friend's removal or block
//! purges their rows (Infoview design §3.2–§3.3).

use crate::row::{query_list, FromRow};
use rekindle_types::activity::{ActivityEntry, ActivityKind};

#[path = "activity_seen.rs"]
pub mod seen;
use rusqlite::{params, Connection, OptionalExtension};

/// The most rows the feed keeps per identity.
pub const MAX_ROWS: i64 = 500;

/// The oldest entry the feed keeps: 30 days, in ms.
pub const MAX_AGE_MS: i64 = 30 * 24 * 60 * 60 * 1000;

/// A stored entry and its row id, the cursor [`page`] takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredActivity {
    pub id: i64,
    pub entry: ActivityEntry,
}

impl FromRow for StoredActivity {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        let kind_column: String = row.get(2)?;
        let payload: String = row.get(3)?;
        let kind: ActivityKind = serde_json::from_str(&payload).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(e))
        })?;
        if kind.name() != kind_column {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                2,
                rusqlite::types::Type::Text,
                format!("kind {kind_column:?} holds a {:?} payload", kind.name()).into(),
            ));
        }
        Ok(StoredActivity {
            id: row.get(0)?,
            entry: ActivityEntry {
                at: row.get(1)?,
                kind,
            },
        })
    }
}

/// Store `entries`, then prune the feed to the newest [`MAX_ROWS`] rows no
/// older than [`MAX_AGE_MS`] before `now_ms`, in one transaction.
///
/// # Errors
/// The write failed, an entry's friend is not a friend row of ours, or an
/// entry would not serialize.
pub fn insert(
    conn: &mut Connection,
    owner_key: &str,
    entries: &[ActivityEntry],
    now_ms: i64,
) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    for entry in entries {
        let payload = serde_json::to_string(&entry.kind)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        tx.execute(
            "INSERT INTO activity_log (owner_key, at, kind, subject_key, payload_json) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                owner_key,
                entry.at,
                entry.kind.name(),
                entry.kind.friend(),
                payload
            ],
        )?;
    }
    prune(&tx, owner_key, now_ms)?;
    tx.commit()
}

/// [`insert`], then the stored entries with their row ids, newest first
/// (what the Host pushes); nothing for no entries.
///
/// # Errors
/// As [`insert`] and [`page`].
pub fn insert_returning(
    conn: &mut Connection,
    owner_key: &str,
    entries: &[ActivityEntry],
    now_ms: i64,
) -> rusqlite::Result<Vec<StoredActivity>> {
    if entries.is_empty() {
        return Ok(Vec::new());
    }
    insert(conn, owner_key, entries, now_ms)?;
    page(
        conn,
        owner_key,
        None,
        u16::try_from(entries.len()).unwrap_or(u16::MAX),
    )
}

/// Drop the entries older than [`MAX_AGE_MS`] before `now_ms`, then all but
/// the newest [`MAX_ROWS`].
///
/// # Errors
/// The write failed.
pub fn prune(conn: &Connection, owner_key: &str, now_ms: i64) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM activity_log WHERE owner_key = ?1 AND at < ?2",
        params![owner_key, now_ms.saturating_sub(MAX_AGE_MS)],
    )?;
    conn.execute(
        "DELETE FROM activity_log WHERE owner_key = ?1 AND id NOT IN \
         (SELECT id FROM activity_log WHERE owner_key = ?1 \
          ORDER BY at DESC, id DESC LIMIT ?2)",
        params![owner_key, MAX_ROWS],
    )?;
    Ok(())
}

/// Up to `limit` entries, the newest first; with `before`, only those
/// after that row in this order (the "View More" cursor). A `before` no
/// longer stored (pruned or purged) answers nothing.
///
/// # Errors
/// The query failed, or a row is corrupt (its payload does not parse or
/// names another kind than its `kind` column).
pub fn page(
    conn: &Connection,
    owner_key: &str,
    before: Option<i64>,
    limit: u16,
) -> rusqlite::Result<Vec<StoredActivity>> {
    let cursor = match before {
        None => None,
        Some(id) => {
            let at: Option<i64> = conn
                .query_row(
                    "SELECT at FROM activity_log WHERE owner_key = ?1 AND id = ?2",
                    params![owner_key, id],
                    |row| row.get(0),
                )
                .optional()?;
            match at {
                Some(at) => Some((at, id)),
                None => return Ok(Vec::new()),
            }
        }
    };
    let (at, id) = cursor.unwrap_or((i64::MAX, i64::MAX));
    query_list(
        conn,
        "SELECT id, at, kind, payload_json FROM activity_log \
         WHERE owner_key = ?1 AND (at, id) < (?2, ?3) \
         ORDER BY at DESC, id DESC LIMIT ?4",
        params![owner_key, at, id, limit],
    )
}

/// Remove every entry about `friend_key` (their removal or block).
///
/// # Errors
/// The write failed.
pub fn purge_friend(conn: &Connection, owner_key: &str, friend_key: &str) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM activity_log WHERE owner_key = ?1 AND subject_key = ?2",
        params![owner_key, friend_key],
    )?;
    Ok(())
}

/// Remove the whole feed (the "Keep an activity log" setting turned off).
///
/// # Errors
/// The write failed.
pub fn clear(conn: &Connection, owner_key: &str) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM activity_log WHERE owner_key = ?1",
        params![owner_key],
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "activity_tests.rs"]
mod tests;
