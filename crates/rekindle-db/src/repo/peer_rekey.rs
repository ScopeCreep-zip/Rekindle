//! A friend rotated their identity (plan E2.3, `IdentityRotated`): their
//! friend row, our direct conversation with them and their
//! `signal_identities` row move from the old key to the new one in one
//! transaction, nothing queued to the old key is kept (it was sealed to a
//! session the new identity does not hold), the old session goes (a
//! record is bound to the identity its AD names), and so does their
//! mailbox key: that record is owned by the old identity, which writes it
//! no more (their route stays on the profile, whose owner carries over).

use rusqlite::{params, Connection};

use super::{identity_text, signal_identities};

/// Move the friend `old` to `new` (both signatures checked by the
/// caller); whether `old` was a friend (nothing moves otherwise).
///
/// # Errors
/// A write failed; nothing moved.
pub fn rotate_peer(
    conn: &Connection,
    owner: &[u8; 32],
    old: &[u8; 32],
    new: &[u8; 32],
    now_ms: i64,
) -> rusqlite::Result<bool> {
    let (owner_key, old_key, new_key) =
        (identity_text(owner), identity_text(old), identity_text(new));
    let tx = conn.unchecked_transaction()?;
    let moved = tx.execute(
        "UPDATE friends SET public_key = ?3, mailbox_dht_key = NULL \
         WHERE owner_key = ?1 AND public_key = ?2",
        params![owner_key, old_key, new_key],
    )? > 0;
    if moved {
        tx.execute(
            "UPDATE messages SET conversation_id = ?3 \
             WHERE owner_key = ?1 AND conversation_type = 'dm' AND conversation_id = ?2",
            params![owner_key, old_key, new_key],
        )?;
        tx.execute(
            "UPDATE messages SET sender_key = ?3 \
             WHERE owner_key = ?1 AND conversation_type = 'dm' AND sender_key = ?2",
            params![owner_key, old_key, new_key],
        )?;
        tx.execute(
            "DELETE FROM pending_messages WHERE owner_key = ?1 AND recipient_key = ?2",
            params![owner_key, old_key],
        )?;
        tx.execute(
            "DELETE FROM ratchet_sessions WHERE owner_key = ?1 AND peer_identity = ?2",
            params![owner_key, old_key],
        )?;
        signal_identities::rotate(&tx, owner, old, new, now_ms)?;
    }
    tx.commit()?;
    Ok(moved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::fixture;

    const OWNER: [u8; 32] = [1; 32];
    const OLD: [u8; 32] = [2; 32];
    const NEW: [u8; 32] = [3; 32];

    #[test]
    fn a_rotated_friend_moves_whole_and_their_old_session_goes() {
        let conn = fixture::identity_conn(&OWNER);
        let (owner, old) = (identity_text(&OWNER), identity_text(&OLD));
        conn.execute(
            "INSERT INTO friends (owner_key, public_key, display_name, added_at, friendship_state, \
             mailbox_dht_key) VALUES (?1, ?2, 'Bob', 1, 'accepted', 'VLD0:old-mailbox')",
            params![owner, old],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages (owner_key, conversation_id, conversation_type, sender_key, \
             body, timestamp) VALUES (?1, ?2, 'dm', ?2, 'hi', 1)",
            params![owner, old],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO pending_messages (owner_key, recipient_key, body, created_at) \
             VALUES (?1, ?2, '{}', 1)",
            params![owner, old],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO ratchet_sessions (owner_key, peer_identity, peer_device, record, updated_at) \
             VALUES (?1, ?2, '', x'00', 1)",
            params![owner, old],
        )
        .unwrap();
        signal_identities::ensure(&conn, &OWNER, &OLD).unwrap();
        signal_identities::mark_verified(&conn, &OWNER, &OLD, 5).unwrap();

        assert!(rotate_peer(&conn, &OWNER, &OLD, &NEW, 9).unwrap());
        let count = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap() };
        let new = identity_text(&NEW);
        assert_eq!(
            count(&format!("SELECT COUNT(*) FROM messages WHERE conversation_id = '{new}' AND sender_key = '{new}'")),
            1
        );
        assert_eq!(
            count(&format!(
                "SELECT COUNT(*) FROM friends WHERE public_key = '{new}'"
            )),
            1
        );
        assert_eq!(
            count("SELECT COUNT(*) FROM friends WHERE mailbox_dht_key IS NOT NULL"),
            0
        );
        assert_eq!(count("SELECT COUNT(*) FROM pending_messages"), 0);
        assert_eq!(count("SELECT COUNT(*) FROM ratchet_sessions"), 0);
        // The verified key changed: the new row is unverified and records it.
        let row = signal_identities::get(&conn, &OWNER, &NEW)
            .unwrap()
            .unwrap();
        assert!(!row.verified);
        assert_eq!(row.changed_from_key, Some(OLD));
        assert!(signal_identities::get(&conn, &OWNER, &OLD)
            .unwrap()
            .is_none());
        assert!(!rotate_peer(&conn, &OWNER, &[9; 32], &[8; 32], 9).unwrap());
    }
}
