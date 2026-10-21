//! `outgoing_invites`: the friend invites we made, tracked so a cancelled
//! one is refused and an answered one is matched to its requester (plan
//! E2.1).

use rekindle_types::dto::friends::OutgoingInvite;
use rusqlite::{params, Connection, OptionalExtension as _};

/// How long an invite we made stays open: the one invite TTL
/// ([`rekindle_types::invite::FRIEND_INVITE_TTL_SECS`], plan E2.5).
pub const INVITE_EXPIRY_MS: i64 =
    rekindle_types::invite::FRIEND_INVITE_TTL_SECS.cast_signed() * 1000;

/// Record a new invite with its request inbox, open for
/// [`INVITE_EXPIRY_MS`]; when it expires.
///
/// # Errors
/// The write failed.
pub fn insert(
    conn: &Connection,
    owner_key: &str,
    invite_id: &str,
    url: &str,
    inbox_key: &str,
    created_at: i64,
) -> rusqlite::Result<i64> {
    let expires_at = created_at + INVITE_EXPIRY_MS;
    conn.execute(
        "INSERT INTO outgoing_invites \
         (owner_key, invite_id, url, inbox_key, created_at, expires_at, status) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending')",
        params![owner_key, invite_id, url, inbox_key, created_at, expires_at],
    )?;
    Ok(expires_at)
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

/// Whether a request from `requester` may answer the invite at `now`: it
/// is ours, still `pending` and not past its expiry, or `responded` by
/// this same requester (their retry). A cancelled, expired, accepted,
/// rejected, unknown, or someone else's answered invite is refused: an
/// invite is single-use (plan E2.5, D17).
///
/// # Errors
/// The query failed.
pub fn admits_request(
    conn: &Connection,
    owner_key: &str,
    invite_id: &str,
    requester: &str,
    now: i64,
) -> rusqlite::Result<bool> {
    let row: Option<(String, i64, Option<String>)> = conn
        .query_row(
            "SELECT status, expires_at, accepted_by FROM outgoing_invites \
             WHERE owner_key = ?1 AND invite_id = ?2",
            params![owner_key, invite_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    Ok(match row {
        Some((status, expires_at, _)) if status == "pending" => now <= expires_at,
        Some((status, _, by)) if status == "responded" => by.as_deref() == Some(requester),
        _ => false,
    })
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

/// Expire the pending invites past their time; their request inboxes,
/// which the caller drops.
///
/// # Errors
/// The write failed.
pub fn expire(conn: &Connection, owner_key: &str, now: i64) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "UPDATE outgoing_invites SET status = 'expired' \
         WHERE owner_key = ?1 AND status = 'pending' AND expires_at < ?2 RETURNING inbox_key",
    )?;
    let rows = stmt
        .query_map(params![owner_key, now], |row| row.get(0))?
        .collect();
    rows
}

/// The request inbox of an invite, any state.
///
/// # Errors
/// The query failed.
pub fn inbox_of(
    conn: &Connection,
    owner_key: &str,
    invite_id: &str,
) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT inbox_key FROM outgoing_invites WHERE owner_key = ?1 AND invite_id = ?2",
        params![owner_key, invite_id],
        |row| row.get(0),
    )
    .optional()
}

/// The request inboxes of the pending invites: the ones the inviter
/// watches.
///
/// # Errors
/// The query failed.
pub fn pending_inboxes(conn: &Connection, owner_key: &str) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT inbox_key FROM outgoing_invites WHERE owner_key = ?1 AND status = 'pending'",
    )?;
    let rows = stmt
        .query_map(params![owner_key], |row| row.get(0))?
        .collect();
    rows
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
        insert(&conn, OWNER, "a", "rekindle://a", "inbox-a", 1).unwrap();
        insert(&conn, OWNER, "b", "rekindle://b", "inbox-b", 2).unwrap();
        mark_responded(&conn, OWNER, "a", "bob").unwrap();
        let open_now = open(&conn, OWNER).unwrap();
        assert_eq!(open_now.len(), 2);
        assert_eq!(open_now[1].accepted_by.as_deref(), Some("bob"));
        mark_accepted(&conn, OWNER, "a").unwrap();
        assert!(cancel(&conn, OWNER, "b").unwrap());
        assert!(!admits_request(&conn, OWNER, "b", "x", 0).unwrap());
        assert!(open(&conn, OWNER).unwrap().is_empty());
        insert(&conn, OWNER, "c", "rekindle://c", "inbox-c", 0).unwrap();
        assert_eq!(
            pending_inboxes(&conn, OWNER).unwrap(),
            vec!["inbox-c".to_string()]
        );
        assert_eq!(
            expire(&conn, OWNER, INVITE_EXPIRY_MS + 1).unwrap(),
            vec!["inbox-c".to_string()]
        );
        assert!(pending_inboxes(&conn, OWNER).unwrap().is_empty());
        assert_eq!(
            inbox_of(&conn, OWNER, "c").unwrap().as_deref(),
            Some("inbox-c")
        );
    }

    #[test]
    fn only_an_open_unexpired_invite_or_its_own_responder_is_admitted() {
        let conn = fixture::conn();
        let expires = insert(&conn, OWNER, "i", "u", "inbox-i", 0).unwrap();
        assert!(admits_request(&conn, OWNER, "i", "bob", expires).unwrap());
        assert!(!admits_request(&conn, OWNER, "i", "bob", expires + 1).unwrap());
        assert!(!admits_request(&conn, OWNER, "unknown", "bob", 0).unwrap());
        mark_responded(&conn, OWNER, "i", "bob").unwrap();
        assert!(admits_request(&conn, OWNER, "i", "bob", 0).unwrap());
        assert!(!admits_request(&conn, OWNER, "i", "carol", 0).unwrap());
        insert(&conn, OWNER, "e", "u", "inbox-e", 0).unwrap();
        expire(&conn, OWNER, expires + 1).unwrap();
        assert!(!admits_request(&conn, OWNER, "e", "bob", 0).unwrap());
    }
}
