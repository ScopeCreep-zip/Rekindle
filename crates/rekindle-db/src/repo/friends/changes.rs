//! Friend changes that touch several rows, each applied whole or not at
//! all (plan audit D-1): one transaction per change, or the caller's when
//! it is already inside one.

use rekindle_types::friend_store::FriendshipState;
use rusqlite::{params, Connection};

use super::{delete, insert_accepted, replace_with_invited_rows, set_record_keys, set_state};
use crate::repo::{blocked, pending_messages, pending_requests};

/// Run `change` atomically: in a transaction of its own, or in the
/// caller's when `conn` is already inside one (a `Transaction` derefs to
/// the connection).
pub(crate) fn atomically<T>(
    conn: &Connection,
    change: impl FnOnce(&Connection) -> rusqlite::Result<T>,
) -> rusqlite::Result<T> {
    if !conn.is_autocommit() {
        return change(conn);
    }
    let tx = conn.unchecked_transaction()?;
    let value = change(&tx)?;
    tx.commit()?;
    Ok(value)
}

/// Delete every ratchet session with `peer`: the friendship that carried
/// them ended. The ratchet tables key both sides by lowercase hex.
fn delete_sessions(conn: &Connection, owner_key: &str, peer: &str) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM ratchet_sessions WHERE owner_key = ?1 AND peer_identity = ?2",
        params![owner_key.to_ascii_lowercase(), peer.to_ascii_lowercase()],
    )?;
    Ok(())
}

/// The record keys a friend names; `None` keeps the stored one.
#[derive(Debug, Clone, Copy, Default)]
pub struct RecordKeys<'a> {
    pub profile: Option<&'a str>,
    pub mailbox: Option<&'a str>,
}

/// We accept `public_key`'s request: the friend stored `accepted` with the
/// records it named, and the request deleted.
///
/// # Errors
/// A write failed; nothing is kept.
pub fn accept_request(
    conn: &Connection,
    owner_key: &str,
    public_key: &str,
    display_name: &str,
    added_at: i64,
    keys: RecordKeys<'_>,
) -> rusqlite::Result<()> {
    atomically(conn, |conn| {
        insert_accepted(conn, owner_key, public_key, display_name, added_at)?;
        pending_requests::delete(conn, owner_key, public_key)?;
        set_record_keys(conn, owner_key, public_key, keys.profile, keys.mailbox)
    })
}

/// Our pending friend `public_key` is accepted (their accept arrived, or
/// both of us asked): `accepted` with the records they named, the name
/// they gave when `Some`, and any request of theirs deleted.
///
/// # Errors
/// A write failed; nothing is kept.
pub fn accept_with_records(
    conn: &Connection,
    owner_key: &str,
    public_key: &str,
    display_name: Option<&str>,
    keys: RecordKeys<'_>,
) -> rusqlite::Result<()> {
    atomically(conn, |conn| {
        set_state(conn, owner_key, public_key, FriendshipState::Accepted)?;
        if let Some(name) = display_name {
            super::set(
                conn,
                owner_key,
                public_key,
                super::Column::DisplayName,
                Some(name.to_string()),
            )?;
        }
        set_record_keys(conn, owner_key, public_key, keys.profile, keys.mailbox)?;
        pending_requests::delete(conn, owner_key, public_key)
    })
}

/// Block `public_key`: the friend, their request, everything queued to
/// them and every session with them go, and the block is stored.
///
/// # Errors
/// A write failed; nothing is kept.
pub fn block(
    conn: &Connection,
    owner_key: &str,
    public_key: &str,
    display_name: &str,
    blocked_at: i64,
) -> rusqlite::Result<()> {
    atomically(conn, |conn| {
        delete(conn, owner_key, public_key)?;
        pending_requests::delete(conn, owner_key, public_key)?;
        pending_messages::delete_to(conn, owner_key, public_key)?;
        delete_sessions(conn, owner_key, public_key)?;
        blocked::insert(conn, owner_key, public_key, display_name, blocked_at)
    })
}

/// We remove `public_key`: the row is `removing`, their request goes, and
/// the `Unfriended` notice (`queued`, the queue's form) is queued to them.
///
/// # Errors
/// A write failed; nothing is kept.
pub fn mark_removing(
    conn: &Connection,
    owner_key: &str,
    public_key: &str,
    queued: &str,
    queued_at: i64,
) -> rusqlite::Result<()> {
    atomically(conn, |conn| {
        set_state(conn, owner_key, public_key, FriendshipState::Removing)?;
        super::clear_presence_key(conn, owner_key, public_key)?;
        pending_requests::delete(conn, owner_key, public_key)?;
        pending_messages::insert(conn, owner_key, public_key, queued, queued_at)
    })
}

/// Forget `public_key`: their row and their request.
///
/// # Errors
/// A write failed; nothing is kept.
pub fn forget(conn: &Connection, owner_key: &str, public_key: &str) -> rusqlite::Result<()> {
    atomically(conn, |conn| {
        delete(conn, owner_key, public_key)?;
        pending_requests::delete(conn, owner_key, public_key)
    })
}

/// `public_key` removed us: their row, their request, everything queued to
/// them and every session with them go.
///
/// # Errors
/// A write failed; nothing is kept.
pub fn unfriended(conn: &Connection, owner_key: &str, public_key: &str) -> rusqlite::Result<()> {
    atomically(conn, |conn| {
        forget(conn, owner_key, public_key)?;
        pending_messages::delete_to(conn, owner_key, public_key)?;
        delete_sessions(conn, owner_key, public_key)
    })
}

/// Forget the friends we removed once nothing is queued to them (their
/// `Unfriended` notice was acknowledged or ran out of retries): their rows
/// and their sessions; their keys.
///
/// # Errors
/// The query or a delete failed; nothing is kept.
pub fn forget_removed(conn: &Connection, owner_key: &str) -> rusqlite::Result<Vec<String>> {
    atomically(conn, |conn| {
        let done = conn
            .prepare(
                "SELECT public_key FROM friends f \
                 WHERE owner_key = ?1 AND friendship_state = 'removing' \
                   AND NOT EXISTS (SELECT 1 FROM pending_messages p \
                                   WHERE p.owner_key = f.owner_key \
                                     AND p.recipient_key = f.public_key)",
            )?
            .query_map(params![owner_key], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        for public_key in &done {
            delete(conn, owner_key, public_key)?;
            delete_sessions(conn, owner_key, public_key)?;
        }
        Ok(done)
    })
}

/// Replace any row for the friend with a `pending_out` row carrying the
/// record keys an invite named.
///
/// # Errors
/// A write failed; nothing is kept.
pub fn replace_with_invited(
    conn: &Connection,
    owner_key: &str,
    public_key: &str,
    display_name: &str,
    added_at: i64,
    profile_key: &str,
    mailbox_key: &str,
) -> rusqlite::Result<()> {
    atomically(conn, |conn| {
        replace_with_invited_rows(
            conn,
            owner_key,
            public_key,
            display_name,
            added_at,
            profile_key,
            mailbox_key,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::fixture::{conn, OWNER};

    const BOB: &str = "bb";

    fn state_of(conn: &Connection) -> Option<String> {
        conn.query_row(
            "SELECT friendship_state FROM friends WHERE owner_key = ?1 AND public_key = ?2",
            params![OWNER, BOB],
            |row| row.get(0),
        )
        .ok()
    }

    fn fail_on(conn: &Connection, table: &str) {
        conn.execute_batch(&format!(
            "CREATE TEMP TRIGGER injected BEFORE INSERT ON {table} \
             BEGIN SELECT RAISE(ABORT, 'injected'); END;"
        ))
        .unwrap();
    }

    #[test]
    fn a_failed_block_keeps_the_friend() {
        let conn = conn();
        insert_accepted(&conn, OWNER, BOB, "Bob", 1).unwrap();
        fail_on(&conn, "blocked_users");
        assert!(block(&conn, OWNER, BOB, "Bob", 2).is_err());
        assert_eq!(state_of(&conn).as_deref(), Some("accepted"));
        conn.execute_batch("DROP TRIGGER injected").unwrap();
        block(&conn, OWNER, BOB, "Bob", 2).unwrap();
        assert_eq!(state_of(&conn), None);
        assert!(blocked::is_blocked(&conn, OWNER, BOB).unwrap());
    }

    #[test]
    fn a_failed_removal_leaves_the_friend_accepted() {
        let conn = conn();
        insert_accepted(&conn, OWNER, BOB, "Bob", 1).unwrap();
        fail_on(&conn, "pending_messages");
        assert!(mark_removing(&conn, OWNER, BOB, "{}", 2).is_err());
        assert_eq!(state_of(&conn).as_deref(), Some("accepted"));
        conn.execute_batch("DROP TRIGGER injected").unwrap();
        mark_removing(&conn, OWNER, BOB, "{}", 2).unwrap();
        assert_eq!(state_of(&conn).as_deref(), Some("removing"));
        assert_eq!(pending_messages::list(&conn, OWNER).unwrap().len(), 1);
    }

    #[test]
    fn a_failed_accept_keeps_the_request_and_the_pending_friend() {
        let conn = conn();
        super::super::insert_pending_out(&conn, OWNER, BOB, "Bob", 1).unwrap();
        conn.execute_batch(
            "CREATE TEMP TRIGGER injected BEFORE UPDATE ON friends \
             WHEN NEW.dht_record_key IS NOT NULL BEGIN SELECT RAISE(ABORT, 'injected'); END;",
        )
        .unwrap();
        let keys = RecordKeys {
            profile: Some("profile"),
            mailbox: None,
        };
        assert!(accept_with_records(&conn, OWNER, BOB, Some("Bobby"), keys).is_err());
        assert_eq!(state_of(&conn).as_deref(), Some("pending_out"));
    }
}
