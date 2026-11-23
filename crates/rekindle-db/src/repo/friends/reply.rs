//! A friend request's reply inbox (plan RV2): `reply_inbox_key` is the one
//! our outgoing request carries, held and watched while the row is
//! `pending_out`; `peer_reply_inbox_key` and `peer_reply_writer` are the
//! one their request carried, kept from our accept until our first contact
//! is committed.

use rusqlite::{params, Connection, OptionalExtension as _};

/// Record `record_key` as the reply inbox our request to `public_key`
/// carries.
///
/// # Errors
/// The write failed.
pub fn set_reply_inbox(
    conn: &Connection,
    owner_key: &str,
    public_key: &str,
    record_key: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE friends SET reply_inbox_key = ?3 WHERE owner_key = ?1 AND public_key = ?2",
        params![owner_key, public_key, record_key],
    )?;
    Ok(())
}

/// The reply inbox our request to `public_key` carries, while the row is
/// `pending_out`.
///
/// # Errors
/// The query failed.
pub fn reply_inbox_of(
    conn: &Connection,
    owner_key: &str,
    public_key: &str,
) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT reply_inbox_key FROM friends \
         WHERE owner_key = ?1 AND public_key = ?2 AND friendship_state = 'pending_out'",
        params![owner_key, public_key],
        |row| row.get::<_, Option<String>>(0),
    )
    .optional()
    .map(Option::flatten)
}

/// Whether our row with `public_key` is `accepted` (plan RV2): the higher
/// side of a crossing, whose request still goes once its reply inbox is
/// released.
///
/// # Errors
/// The query failed.
pub fn is_accepted_row(
    conn: &Connection,
    owner_key: &str,
    public_key: &str,
) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM friends \
         WHERE owner_key = ?1 AND public_key = ?2 AND friendship_state = 'accepted')",
        params![owner_key, public_key],
        |row| row.get(0),
    )
}

/// The reply inboxes of our open requests (rows still `pending_out`), in
/// key order: the ones held and watched.
///
/// # Errors
/// The query failed.
pub fn reply_inboxes(conn: &Connection, owner_key: &str) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT reply_inbox_key FROM friends \
         WHERE owner_key = ?1 AND friendship_state = 'pending_out' \
           AND reply_inbox_key IS NOT NULL ORDER BY reply_inbox_key",
    )?;
    let keys = stmt
        .query_map(params![owner_key], |row| row.get(0))?
        .collect();
    keys
}

/// Keep the reply inbox `public_key`'s request carried (`record_key`) and
/// its one-time `writer`, for our first contact.
///
/// # Errors
/// The write failed.
pub fn set_peer_reply(
    conn: &Connection,
    owner_key: &str,
    public_key: &str,
    record_key: &str,
    writer: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE friends SET peer_reply_inbox_key = ?3, peer_reply_writer = ?4 \
         WHERE owner_key = ?1 AND public_key = ?2",
        params![owner_key, public_key, record_key, writer],
    )?;
    Ok(())
}

/// Take `public_key`'s reply inbox and writer: read and cleared in one
/// transaction, so one first contact is written there at most.
///
/// # Errors
/// The query or the write failed.
pub fn take_peer_reply(
    conn: &Connection,
    owner_key: &str,
    public_key: &str,
) -> rusqlite::Result<Option<(String, String)>> {
    super::changes::atomically(conn, |conn| {
        let held = conn
            .query_row(
                "SELECT peer_reply_inbox_key, peer_reply_writer FROM friends \
                 WHERE owner_key = ?1 AND public_key = ?2 AND peer_reply_inbox_key IS NOT NULL",
                params![owner_key, public_key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if held.is_some() {
            conn.execute(
                "UPDATE friends SET peer_reply_inbox_key = NULL, peer_reply_writer = NULL \
                 WHERE owner_key = ?1 AND public_key = ?2",
                params![owner_key, public_key],
            )?;
        }
        Ok(held)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::fixture::{self, OWNER};
    use crate::repo::friends::{insert_accepted, insert_pending_out, set_state};
    use rekindle_types::friend::FriendshipState;

    #[test]
    fn only_open_requests_hold_their_reply_inbox() {
        let conn = fixture::conn();
        insert_pending_out(&conn, OWNER, "ada", "Ada", 1).unwrap();
        insert_pending_out(&conn, OWNER, "bo", "Bo", 1).unwrap();
        set_reply_inbox(&conn, OWNER, "ada", "VLD0:a").unwrap();
        set_reply_inbox(&conn, OWNER, "bo", "VLD0:b").unwrap();
        assert_eq!(reply_inboxes(&conn, OWNER).unwrap(), ["VLD0:a", "VLD0:b"]);
        set_state(&conn, OWNER, "bo", FriendshipState::Accepted).unwrap();
        assert_eq!(reply_inboxes(&conn, OWNER).unwrap(), ["VLD0:a"]);
        assert!(is_accepted_row(&conn, OWNER, "bo").unwrap());
        assert!(!is_accepted_row(&conn, OWNER, "ada").unwrap());
        assert!(!is_accepted_row(&conn, OWNER, "nobody").unwrap());
    }

    #[test]
    fn a_peer_reply_is_taken_once() {
        let conn = fixture::conn();
        insert_accepted(&conn, OWNER, "cy", "Cy", 1).unwrap();
        assert_eq!(take_peer_reply(&conn, OWNER, "cy").unwrap(), None);
        set_peer_reply(&conn, OWNER, "cy", "VLD0:c", "VLD0:p:s").unwrap();
        assert_eq!(
            take_peer_reply(&conn, OWNER, "cy").unwrap(),
            Some(("VLD0:c".to_owned(), "VLD0:p:s".to_owned()))
        );
        assert_eq!(take_peer_reply(&conn, OWNER, "cy").unwrap(), None);
    }
}
