//! `pending_messages`: signed 1:1 envelopes waiting for a route (plan
//! E2.1; E2.5's outbox replaces them, D22). The body is the envelope's
//! JSON; a retry re-signs it with a fresh timestamp.

use rusqlite::{params, Connection};

/// One queued envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub id: i64,
    pub recipient_key: String,
    pub body: String,
    pub retry_count: i64,
}

/// Queue `body` for `recipient_key`.
///
/// # Errors
/// The write failed.
pub fn insert(
    conn: &Connection,
    owner_key: &str,
    recipient_key: &str,
    body: &str,
    created_at: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO pending_messages (owner_key, recipient_key, body, created_at) \
         VALUES (?1, ?2, ?3, ?4)",
        params![owner_key, recipient_key, body, created_at],
    )?;
    Ok(())
}

/// Every queued envelope, oldest first.
///
/// # Errors
/// The query failed.
pub fn list(conn: &Connection, owner_key: &str) -> rusqlite::Result<Vec<Pending>> {
    let mut stmt = conn.prepare(
        "SELECT id, recipient_key, body, retry_count FROM pending_messages \
         WHERE owner_key = ?1 ORDER BY id",
    )?;
    let rows = stmt
        .query_map(params![owner_key], |row| {
            Ok(Pending {
                id: row.get(0)?,
                recipient_key: row.get(1)?,
                body: row.get(2)?,
                retry_count: row.get(3)?,
            })
        })?
        .collect();
    rows
}

/// Drop the queued envelope `id` (delivered, or out of retries).
///
/// # Errors
/// The write failed.
pub fn delete(conn: &Connection, owner_key: &str, id: i64) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM pending_messages WHERE owner_key = ?1 AND id = ?2",
        params![owner_key, id],
    )?;
    Ok(())
}

/// Count one more failed attempt for `id`.
///
/// # Errors
/// The write failed.
pub fn bump_retry(conn: &Connection, owner_key: &str, id: i64) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE pending_messages SET retry_count = retry_count + 1 \
         WHERE owner_key = ?1 AND id = ?2",
        params![owner_key, id],
    )?;
    Ok(())
}

/// Drop everything queued to `recipient_key` (they unfriended us, or
/// acknowledged our removal, or we blocked them).
///
/// # Errors
/// The write failed.
pub fn delete_to(conn: &Connection, owner_key: &str, recipient_key: &str) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM pending_messages WHERE owner_key = ?1 AND recipient_key = ?2",
        params![owner_key, recipient_key],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::fixture::{self, OWNER};

    #[test]
    fn a_queued_envelope_retries_until_it_is_dropped() {
        let conn = fixture::conn();
        insert(&conn, OWNER, "bob", "{}", 1).unwrap();
        insert(&conn, OWNER, "carol", "{}", 2).unwrap();
        let first = list(&conn, OWNER).unwrap()[0].id;
        bump_retry(&conn, OWNER, first).unwrap();
        assert_eq!(list(&conn, OWNER).unwrap()[0].retry_count, 1);
        delete(&conn, OWNER, first).unwrap();
        delete_to(&conn, OWNER, "carol").unwrap();
        assert!(list(&conn, OWNER).unwrap().is_empty());
    }
}
