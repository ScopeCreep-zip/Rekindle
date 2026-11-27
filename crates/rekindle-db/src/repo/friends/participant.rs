//! The dm participant records of each conversation (plan E2.5, MSR): the
//! `participant_record_key` (ours) and `peer_participant_record_key`
//! (theirs) columns of `friends`.

use crate::row::query_list;
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
    query_list(
        conn,
        "SELECT public_key, peer_participant_record_key FROM friends \
         WHERE owner_key = ?1 AND friendship_state = 'accepted' \
         AND peer_participant_record_key IS NOT NULL AND (?2 IS NULL OR public_key = ?2)",
        params![owner_key, public_key],
    )
}

#[cfg(test)]
#[path = "participant_tests.rs"]
mod tests;
