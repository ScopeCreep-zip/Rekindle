//! The dm participant records of each conversation (plan E2.5, MSR): the
//! `participant_record_key` (ours) and `peer_participant_record_key`
//! (theirs) columns of `friends`.

use rusqlite::{params, Connection, OptionalExtension as _};

/// Store our participant record for the conversation with `public_key`.
/// Whether a friend row took it.
///
/// A new record is a new chain, numbered from 1: the `chain_entries` of
/// the record it replaces (an ended friendship's, or the one before an
/// identity rotation) go with it, so a cumulative `DmAck.through` on the
/// new chain maps to the new chain's rows only (live A/B finding 71: a
/// re-friend's entries were not noted under the old chain's seqs).
///
/// # Errors
/// The write failed; nothing changed.
pub fn set_participant_record_key(
    conn: &Connection,
    owner_key: &str,
    public_key: &str,
    record_key: &str,
) -> rusqlite::Result<bool> {
    let tx = conn.unchecked_transaction()?;
    let changed = tx.execute(
        "UPDATE friends SET participant_record_key = ?3, dm_chain_head = 0 \
         WHERE owner_key = ?1 AND public_key = ?2 \
           AND participant_record_key IS NOT ?3",
        params![owner_key, public_key, record_key],
    )?;
    if changed > 0 {
        tx.execute(
            "DELETE FROM chain_entries WHERE owner_key = ?1 AND peer = ?2",
            params![owner_key, public_key],
        )?;
    }
    let held = changed > 0
        || participant_record_key(&tx, owner_key, public_key)?.as_deref() == Some(record_key);
    tx.commit()?;
    Ok(held)
}

/// Store `public_key`'s participant record (their `StreamRecord`), for an
/// accepted friend only. Whether it changed the row.
///
/// # Errors
/// The write failed.
pub fn set_peer_participant_record_key(
    conn: &Connection,
    owner_key: &str,
    public_key: &str,
    record_key: &str,
) -> rusqlite::Result<bool> {
    let changed = conn.execute(
        "UPDATE friends SET peer_participant_record_key = ?3 \
         WHERE owner_key = ?1 AND public_key = ?2 AND friendship_state = 'accepted' \
         AND peer_participant_record_key IS NOT ?3",
        params![owner_key, public_key, record_key],
    )?;
    Ok(changed > 0)
}

/// Our participant record for the conversation with `public_key`.
///
/// # Errors
/// The query failed.
pub fn participant_record_key(
    conn: &Connection,
    owner_key: &str,
    public_key: &str,
) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT participant_record_key FROM friends WHERE owner_key = ?1 AND public_key = ?2",
        params![owner_key, public_key],
        |row| row.get(0),
    )
    .optional()
    .map(Option::flatten)
}

/// `(public_key, record_key)` of every accepted friend whose participant
/// record we hold, or `public_key`'s alone when given.
///
/// # Errors
/// The query failed.
pub fn peer_participant_record_key(
    conn: &Connection,
    owner_key: &str,
    public_key: Option<&str>,
) -> rusqlite::Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT public_key, peer_participant_record_key FROM friends \
         WHERE owner_key = ?1 AND friendship_state = 'accepted' \
         AND peer_participant_record_key IS NOT NULL AND (?2 IS NULL OR public_key = ?2)",
    )?;
    let rows = stmt
        .query_map(params![owner_key, public_key], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?
        .collect();
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::fixture::{self, OWNER};

    #[test]
    fn a_peer_record_is_kept_for_an_accepted_friend_only() {
        let conn = fixture::conn();
        conn.execute(
            "INSERT INTO friends (owner_key, public_key, added_at, friendship_state) \
             VALUES (?1, 'acc', 0, 'accepted'), (?1, 'pend', 0, 'pending_out')",
            params![OWNER],
        )
        .unwrap();
        assert!(set_peer_participant_record_key(&conn, OWNER, "acc", "r1").unwrap());
        assert!(!set_peer_participant_record_key(&conn, OWNER, "acc", "r1").unwrap());
        assert!(!set_peer_participant_record_key(&conn, OWNER, "pend", "r2").unwrap());
        assert_eq!(
            peer_participant_record_key(&conn, OWNER, None).unwrap(),
            [("acc".to_string(), "r1".to_string())]
        );
        assert!(set_participant_record_key(&conn, OWNER, "pend", "ours").unwrap());
        assert_eq!(
            participant_record_key(&conn, OWNER, "pend")
                .unwrap()
                .as_deref(),
            Some("ours")
        );
        assert_eq!(participant_record_key(&conn, OWNER, "none").unwrap(), None);
    }

    #[test]
    fn a_new_record_starts_a_new_chain() {
        let conn = fixture::conn();
        conn.execute(
            "INSERT INTO friends (owner_key, public_key, added_at, friendship_state) \
             VALUES (?1, 'bob', 0, 'accepted')",
            params![OWNER],
        )
        .unwrap();
        assert!(set_participant_record_key(&conn, OWNER, "bob", "old").unwrap());
        crate::repo::chain_entries::record(&conn, OWNER, "bob", 1, 1, 0).unwrap();
        // The same record again keeps its chain.
        assert!(set_participant_record_key(&conn, OWNER, "bob", "old").unwrap());
        let count = |c: &Connection| -> i64 {
            c.query_row("SELECT COUNT(*) FROM chain_entries", [], |r| r.get(0))
                .unwrap()
        };
        assert_eq!(count(&conn), 1);
        // A new one forgets the old chain's entries, so its seq 1 is noted.
        assert!(set_participant_record_key(&conn, OWNER, "bob", "new").unwrap());
        assert_eq!(count(&conn), 0);
        crate::repo::chain_entries::record(&conn, OWNER, "bob", 1, 9, 0).unwrap();
        let seq: i64 = conn
            .query_row(
                "SELECT stream_seq FROM chain_entries WHERE chain_seq = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(seq, 9);
    }
}
