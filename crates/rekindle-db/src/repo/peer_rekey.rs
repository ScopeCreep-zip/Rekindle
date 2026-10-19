//! A friend rotated their identity (plan E2.3, `IdentityRotated`): their
//! friend row and our direct conversation with them move from the old key
//! to the new one, and nothing queued to the old key is kept (it was
//! sealed to a session the new identity does not hold). The verification
//! bookkeeping is `signal_identities::rotate`'s.

use rusqlite::{params, Connection};

/// Move `old`'s friend row and DM history to `new`; whether `old` was a
/// friend (nothing moves otherwise).
///
/// # Errors
/// A write failed; nothing moved.
pub fn rekey_peer(
    conn: &Connection,
    owner_key: &str,
    old: &str,
    new: &str,
) -> rusqlite::Result<bool> {
    let tx = conn.unchecked_transaction()?;
    let moved = tx.execute(
        "UPDATE friends SET public_key = ?3 WHERE owner_key = ?1 AND public_key = ?2",
        params![owner_key, old, new],
    )? > 0;
    if moved {
        tx.execute(
            "UPDATE messages SET conversation_id = ?3 \
             WHERE owner_key = ?1 AND conversation_type = 'dm' AND conversation_id = ?2",
            params![owner_key, old, new],
        )?;
        tx.execute(
            "UPDATE messages SET sender_key = ?3 \
             WHERE owner_key = ?1 AND conversation_type = 'dm' AND sender_key = ?2",
            params![owner_key, old, new],
        )?;
        tx.execute(
            "DELETE FROM pending_messages WHERE owner_key = ?1 AND recipient_key = ?2",
            params![owner_key, old],
        )?;
    }
    tx.commit()?;
    Ok(moved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::fixture::{self, OWNER};

    #[test]
    fn a_rotated_friend_keeps_their_row_and_history() {
        let conn = fixture::conn();
        conn.execute(
            "INSERT INTO friends (owner_key, public_key, display_name, added_at, friendship_state) \
             VALUES (?1, 'old', 'Bob', 1, 'accepted')",
            [OWNER],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages (owner_key, conversation_id, conversation_type, sender_key, \
             body, timestamp) VALUES (?1, 'old', 'dm', 'old', 'hi', 1)",
            [OWNER],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO pending_messages (owner_key, recipient_key, body, created_at) \
             VALUES (?1, 'old', '{}', 1)",
            [OWNER],
        )
        .unwrap();
        assert!(rekey_peer(&conn, OWNER, "old", "new").unwrap());
        let (conversation, sender): (String, String) = conn
            .query_row(
                "SELECT conversation_id, sender_key FROM messages",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((conversation.as_str(), sender.as_str()), ("new", "new"));
        let friend: String = conn
            .query_row("SELECT public_key FROM friends", [], |r| r.get(0))
            .unwrap();
        assert_eq!(friend, "new");
        let queued: i64 = conn
            .query_row("SELECT COUNT(*) FROM pending_messages", [], |r| r.get(0))
            .unwrap();
        assert_eq!(queued, 0);
        assert!(!rekey_peer(&conn, OWNER, "stranger", "x").unwrap());
    }
}
