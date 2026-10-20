//! The session resets we asked friends for (plan audit D-4): only a friend
//! we asked may open a session with `SessionResetAccept` (B11). Stored, so
//! a lock or a restart does not lose the request; consumed inside the
//! transaction that accepts the friend's init (`ratchet::accept_init`).

use rusqlite::{params, Connection};

/// Record that we asked `peer` to reset our session (the newest wins).
///
/// # Errors
/// The write failed.
pub fn insert(
    conn: &Connection,
    owner_key: &str,
    peer: &str,
    requested_at: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO session_reset_requests (owner_key, peer, requested_at) \
         VALUES (?1, ?2, ?3)",
        params![owner_key, peer, requested_at],
    )?;
    Ok(())
}

/// Delete our request to `peer` (accepted or declined); whether one was
/// outstanding.
///
/// # Errors
/// The write failed.
pub fn take(conn: &Connection, owner_key: &str, peer: &str) -> rusqlite::Result<bool> {
    Ok(conn.execute(
        "DELETE FROM session_reset_requests WHERE owner_key = ?1 AND peer = ?2",
        params![owner_key, peer],
    )? > 0)
}

/// Delete the requests made before `cutoff` (unanswered past the
/// backend-hold lifetime); how many.
///
/// # Errors
/// The write failed.
pub fn delete_before(conn: &Connection, owner_key: &str, cutoff: i64) -> rusqlite::Result<usize> {
    conn.execute(
        "DELETE FROM session_reset_requests WHERE owner_key = ?1 AND requested_at < ?2",
        params![owner_key, cutoff],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::fixture::{conn, OWNER};

    #[test]
    fn a_request_is_taken_once() {
        let conn = conn();
        insert(&conn, OWNER, "bb", 10).unwrap();
        assert!(take(&conn, OWNER, "bb").unwrap());
        assert!(!take(&conn, OWNER, "bb").unwrap(), "consumed");
        assert!(!take(&conn, OWNER, "cc").unwrap(), "never asked");
    }

    #[test]
    fn requests_past_the_lifetime_expire() {
        let conn = conn();
        insert(&conn, OWNER, "old", 10).unwrap();
        insert(&conn, OWNER, "new", 100).unwrap();
        assert_eq!(delete_before(&conn, OWNER, 50).unwrap(), 1);
        assert!(!take(&conn, OWNER, "old").unwrap());
        assert!(take(&conn, OWNER, "new").unwrap());
    }
}
