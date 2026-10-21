//! The dm participant records of each conversation (plan E2.5, MSR): the
//! `participant_record_key` (ours) and `peer_participant_record_key`
//! (theirs) columns of `friends`.

use rusqlite::{params, Connection, OptionalExtension as _};

/// Store our participant record for the conversation with `public_key`.
/// Whether a friend row took it.
///
/// # Errors
/// The write failed.
pub fn set_participant_record_key(
    conn: &Connection,
    owner_key: &str,
    public_key: &str,
    record_key: &str,
) -> rusqlite::Result<bool> {
    let changed = conn.execute(
        "UPDATE friends SET participant_record_key = ?3 \
         WHERE owner_key = ?1 AND public_key = ?2",
        params![owner_key, public_key, record_key],
    )?;
    Ok(changed > 0)
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
}
