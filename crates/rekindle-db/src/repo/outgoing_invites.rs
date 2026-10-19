//! `outgoing_invites`: the friend invites we made, tracked so a cancelled
//! one is refused and an answered one is matched to its requester (plan
//! E2.1).

use rekindle_types::dto::friends::OutgoingInvite;
use rusqlite::{params, Connection, OptionalExtension as _};

/// How long an invite we made stays open: 48 hours.
pub const INVITE_EXPIRY_MS: i64 = 48 * 60 * 60 * 1000;

/// Record a new invite, open for [`INVITE_EXPIRY_MS`].
///
/// # Errors
/// The write failed.
pub fn insert(
    conn: &Connection,
    owner_key: &str,
    invite_id: &str,
    url: &str,
    created_at: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO outgoing_invites (owner_key, invite_id, url, created_at, expires_at, status) \
         VALUES (?1, ?2, ?3, ?4, ?5, 'pending')",
        params![
            owner_key,
            invite_id,
            url,
            created_at,
            created_at + INVITE_EXPIRY_MS
        ],
    )?;
    Ok(())
}

/// Cancel a pending invite; whether one was.
///
/// # Errors
/// The write failed.
pub fn cancel(conn: &Connection, owner_key: &str, invite_id: &str) -> rusqlite::Result<bool> {
    Ok(conn.execute(
        "UPDATE outgoing_invites SET status = 'cancelled' \
         WHERE owner_key = ?1 AND invite_id = ?2 AND status = 'pending'",
        params![owner_key, invite_id],
    )? > 0)
}

/// Whether the invite was cancelled (a request answering it is refused).
///
/// # Errors
/// The query failed.
pub fn is_cancelled(conn: &Connection, owner_key: &str, invite_id: &str) -> rusqlite::Result<bool> {
    let status: Option<String> = conn
        .query_row(
            "SELECT status FROM outgoing_invites WHERE owner_key = ?1 AND invite_id = ?2",
            params![owner_key, invite_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(status.as_deref() == Some("cancelled"))
}

/// A pending invite was answered by `responder_key`'s request.
///
/// # Errors
/// The write failed.
pub fn mark_responded(
    conn: &Connection,
    owner_key: &str,
    invite_id: &str,
    responder_key: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE outgoing_invites SET status = 'responded', accepted_by = ?3 \
         WHERE owner_key = ?1 AND invite_id = ?2 AND status = 'pending'",
        params![owner_key, invite_id, responder_key],
    )?;
    Ok(())
}

/// We accepted the request that answered the invite.
///
/// # Errors
/// The write failed.
pub fn mark_accepted(conn: &Connection, owner_key: &str, invite_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE outgoing_invites SET status = 'accepted' \
         WHERE owner_key = ?1 AND invite_id = ?2 AND status = 'responded'",
        params![owner_key, invite_id],
    )?;
    Ok(())
}

/// We rejected the request that answered the invite.
///
/// # Errors
/// The write failed.
pub fn mark_rejected(conn: &Connection, owner_key: &str, invite_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE outgoing_invites SET status = 'rejected' WHERE owner_key = ?1 AND invite_id = ?2",
        params![owner_key, invite_id],
    )?;
    Ok(())
}

/// Expire the pending invites past their time; how many.
///
/// # Errors
/// The write failed.
pub fn expire(conn: &Connection, owner_key: &str, now: i64) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE outgoing_invites SET status = 'expired' \
         WHERE owner_key = ?1 AND status = 'pending' AND expires_at < ?2",
        params![owner_key, now],
    )
}

/// The open invites (pending or responded), newest first.
///
/// # Errors
/// The query failed.
pub fn open(conn: &Connection, owner_key: &str) -> rusqlite::Result<Vec<OutgoingInvite>> {
    let mut stmt = conn.prepare(
        "SELECT invite_id, url, created_at, expires_at, status, accepted_by \
         FROM outgoing_invites WHERE owner_key = ?1 AND status IN ('pending', 'responded') \
         ORDER BY created_at DESC",
    )?;
    let rows = stmt
        .query_map(params![owner_key], |row| {
            Ok(OutgoingInvite {
                invite_id: row.get(0)?,
                url: row.get(1)?,
                created_at: row.get(2)?,
                expires_at: row.get(3)?,
                status: row.get(4)?,
                accepted_by: row.get(5)?,
            })
        })?
        .collect();
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::fixture::{self, OWNER};

    #[test]
    fn an_invite_moves_through_its_states() {
        let conn = fixture::conn();
        insert(&conn, OWNER, "a", "rekindle://a", 1).unwrap();
        insert(&conn, OWNER, "b", "rekindle://b", 2).unwrap();
        mark_responded(&conn, OWNER, "a", "bob").unwrap();
        let open_now = open(&conn, OWNER).unwrap();
        assert_eq!(open_now.len(), 2);
        assert_eq!(open_now[1].accepted_by.as_deref(), Some("bob"));
        mark_accepted(&conn, OWNER, "a").unwrap();
        assert!(cancel(&conn, OWNER, "b").unwrap());
        assert!(is_cancelled(&conn, OWNER, "b").unwrap());
        assert!(open(&conn, OWNER).unwrap().is_empty());
        insert(&conn, OWNER, "c", "rekindle://c", 0).unwrap();
        assert_eq!(expire(&conn, OWNER, INVITE_EXPIRY_MS + 1).unwrap(), 1);
    }
}
