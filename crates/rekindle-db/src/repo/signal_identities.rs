//! `signal_identities`: each peer's identity key and whether the user
//! verified it (plan E2.3, "Safety numbers and verification").
//!
//! Under identity == address a peer's key changes only when the peer
//! rotates its identity (`IdentityRotated`, 0x0108). [`rotate`] moves the
//! row to the new key and clears `verified`, as Signal resets verification
//! on a safety-number change; when the row was verified it also records
//! the change (`changed_from_key`, `changed_at`), and while that is set
//! [`verified_key_changed`] raises the `VerifiedKeyChanged` escalation.
//! Committing the escalated action acknowledges it ([`acknowledging`], in
//! the action's own transaction, or [`acknowledge_key_change`] after an
//! action whose write is not here); marking the new key verified does too.

use rekindle_crypto::SafetyNumber;
use rekindle_types::intent::Escalation;
use rusqlite::{params, Connection, OptionalExtension};

use super::identity_text;

/// One peer's row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalIdentity {
    pub identity_key: [u8; 32],
    pub verified: bool,
    pub verified_at: Option<i64>,
    /// The verified key this one replaced, until acknowledged.
    pub changed_from_key: Option<[u8; 32]>,
    pub changed_at: Option<i64>,
}

/// `peer`'s row, if we hold one.
///
/// # Errors
/// The query failed or the row is corrupt.
pub fn get(
    conn: &Connection,
    owner: &[u8; 32],
    peer: &[u8; 32],
) -> rusqlite::Result<Option<SignalIdentity>> {
    conn.query_row(
        "SELECT identity_key, verified, verified_at, changed_from_key, changed_at \
         FROM signal_identities WHERE owner_key = ?1 AND peer_identity = ?2",
        params![identity_text(owner), identity_text(peer)],
        |row| {
            Ok(SignalIdentity {
                identity_key: row.get(0)?,
                verified: row.get(1)?,
                verified_at: row.get(2)?,
                changed_from_key: row.get(3)?,
                changed_at: row.get(4)?,
            })
        },
    )
    .optional()
}

/// Record `peer`'s key on first contact (trust on first use, unverified);
/// an existing row is kept.
pub(crate) fn ensure(conn: &Connection, owner: &[u8; 32], peer: &[u8; 32]) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO signal_identities (owner_key, peer_identity, identity_key) \
         VALUES (?1, ?2, ?3)",
        params![identity_text(owner), identity_text(peer), peer],
    )?;
    Ok(())
}

/// The user compared the safety number and marks `peer` verified; an
/// unacknowledged key change is acknowledged with it. `false` when we
/// hold no row for `peer`.
///
/// # Errors
/// The write failed.
pub fn mark_verified(
    conn: &Connection,
    owner: &[u8; 32],
    peer: &[u8; 32],
    now_ms: i64,
) -> rusqlite::Result<bool> {
    let changed = conn.execute(
        "UPDATE signal_identities SET verified = 1, verified_at = ?3, changed_from_key = NULL, \
         changed_at = NULL WHERE owner_key = ?1 AND peer_identity = ?2",
        params![identity_text(owner), identity_text(peer), now_ms],
    )?;
    Ok(changed == 1)
}

/// The user withdraws `peer`'s verification. `false` when we hold no row.
///
/// # Errors
/// The write failed.
pub fn clear_verification(
    conn: &Connection,
    owner: &[u8; 32],
    peer: &[u8; 32],
) -> rusqlite::Result<bool> {
    let changed = conn.execute(
        "UPDATE signal_identities SET verified = 0, verified_at = NULL \
         WHERE owner_key = ?1 AND peer_identity = ?2",
        params![identity_text(owner), identity_text(peer)],
    )?;
    Ok(changed == 1)
}

/// The user committed an action escalated by [`verified_key_changed`]:
/// the change is acknowledged; `verified` stays cleared until the user
/// marks the new key verified. Run it in the action's own transaction.
///
/// # Errors
/// The write failed.
pub fn acknowledge_key_change(
    conn: &Connection,
    owner: &[u8; 32],
    peer: &[u8; 32],
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE signal_identities SET changed_from_key = NULL, changed_at = NULL \
         WHERE owner_key = ?1 AND peer_identity = ?2",
        params![identity_text(owner), identity_text(peer)],
    )?;
    Ok(())
}

/// Run `write`, the own write of an action whose review a
/// [`verified_key_changed`] escalation held, and acknowledge `peer`'s key
/// change in the same transaction: the action and the acknowledgement
/// commit together or not at all.
///
/// # Errors
/// `write` or the acknowledgement failed; neither is kept.
pub fn acknowledging<R>(
    conn: &Connection,
    owner: &[u8; 32],
    peer: &[u8; 32],
    write: impl FnOnce(&Connection) -> rusqlite::Result<R>,
) -> rusqlite::Result<R> {
    let tx = conn.unchecked_transaction()?;
    let written = write(&tx)?;
    acknowledge_key_change(&tx, owner, peer)?;
    tx.commit()?;
    Ok(written)
}

/// A friend rotated from `old` to `new` (both signatures checked by the
/// caller): the row moves to `new`, unverified. A verified row records the
/// change; an unacknowledged earlier change keeps its first key, the one
/// the user last verified.
///
/// # Errors
/// The write failed.
pub fn rotate(
    conn: &Connection,
    owner: &[u8; 32],
    old: &[u8; 32],
    new: &[u8; 32],
    now_ms: i64,
) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    let previous = get(&tx, owner, old)?;
    let (changed_from, changed_at) = match &previous {
        Some(row) if row.changed_from_key.is_some() => (row.changed_from_key, row.changed_at),
        Some(row) if row.verified => (Some(*old), Some(now_ms)),
        _ => (None, None),
    };
    tx.execute(
        "DELETE FROM signal_identities WHERE owner_key = ?1 AND peer_identity = ?2",
        params![identity_text(owner), identity_text(old)],
    )?;
    tx.execute(
        "INSERT OR REPLACE INTO signal_identities (owner_key, peer_identity, identity_key, \
         verified, verified_at, changed_from_key, changed_at) VALUES (?1, ?2, ?3, 0, NULL, ?4, ?5)",
        params![
            identity_text(owner),
            identity_text(new),
            new,
            changed_from,
            changed_at
        ],
    )?;
    tx.commit()
}

/// `VerifiedKeyChanged` while `peer`'s verified key changed and the change
/// is unacknowledged: the safety numbers of the old key and the new one.
///
/// # Errors
/// The query failed or the row is corrupt.
pub fn verified_key_changed(
    conn: &Connection,
    owner: &[u8; 32],
    peer: &[u8; 32],
) -> rusqlite::Result<Option<Escalation>> {
    Ok(get(conn, owner, peer)?.and_then(|row| {
        row.changed_from_key
            .map(|old| Escalation::VerifiedKeyChanged {
                old_safety_number: SafetyNumber::new(owner, &old).digits(),
                new_safety_number: SafetyNumber::new(owner, &row.identity_key).digits(),
            })
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::fixture;

    const OWNER: [u8; 32] = [1; 32];
    const OLD: [u8; 32] = [2; 32];
    const NEW: [u8; 32] = [3; 32];

    #[test]
    fn first_contact_is_unverified_and_kept() {
        let conn = fixture::identity_conn(&OWNER);
        assert_eq!(get(&conn, &OWNER, &OLD).unwrap(), None);
        ensure(&conn, &OWNER, &OLD).unwrap();
        mark_verified(&conn, &OWNER, &OLD, 5).unwrap();
        ensure(&conn, &OWNER, &OLD).unwrap();
        let row = get(&conn, &OWNER, &OLD).unwrap().unwrap();
        assert_eq!(row.identity_key, OLD);
        assert!(row.verified, "a later contact keeps the row");
        assert_eq!(row.verified_at, Some(5));
    }

    #[test]
    fn verification_is_marked_and_cleared() {
        let conn = fixture::identity_conn(&OWNER);
        assert!(!mark_verified(&conn, &OWNER, &OLD, 1).unwrap());
        ensure(&conn, &OWNER, &OLD).unwrap();
        assert!(mark_verified(&conn, &OWNER, &OLD, 1).unwrap());
        assert!(clear_verification(&conn, &OWNER, &OLD).unwrap());
        let row = get(&conn, &OWNER, &OLD).unwrap().unwrap();
        assert!(!row.verified);
        assert_eq!(row.verified_at, None);
    }

    #[test]
    fn rotating_an_unverified_friend_records_no_change() {
        let conn = fixture::identity_conn(&OWNER);
        ensure(&conn, &OWNER, &OLD).unwrap();
        rotate(&conn, &OWNER, &OLD, &NEW, 9).unwrap();
        assert_eq!(get(&conn, &OWNER, &OLD).unwrap(), None);
        let row = get(&conn, &OWNER, &NEW).unwrap().unwrap();
        assert_eq!((row.identity_key, row.changed_from_key), (NEW, None));
        assert_eq!(verified_key_changed(&conn, &OWNER, &NEW).unwrap(), None);
    }

    #[test]
    fn a_verified_key_change_escalates_until_acknowledged() {
        let conn = fixture::identity_conn(&OWNER);
        ensure(&conn, &OWNER, &OLD).unwrap();
        mark_verified(&conn, &OWNER, &OLD, 1).unwrap();
        rotate(&conn, &OWNER, &OLD, &NEW, 9).unwrap();

        let row = get(&conn, &OWNER, &NEW).unwrap().unwrap();
        assert!(!row.verified, "verification resets on a key change");
        assert_eq!((row.changed_from_key, row.changed_at), (Some(OLD), Some(9)));
        let Some(Escalation::VerifiedKeyChanged {
            old_safety_number,
            new_safety_number,
        }) = verified_key_changed(&conn, &OWNER, &NEW).unwrap()
        else {
            panic!("a verified key change escalates");
        };
        assert_eq!(old_safety_number, SafetyNumber::new(&OWNER, &OLD).digits());
        assert_eq!(new_safety_number, SafetyNumber::new(&OWNER, &NEW).digits());
        assert_ne!(old_safety_number, new_safety_number);

        acknowledge_key_change(&conn, &OWNER, &NEW).unwrap();
        assert_eq!(verified_key_changed(&conn, &OWNER, &NEW).unwrap(), None);
        assert!(!get(&conn, &OWNER, &NEW).unwrap().unwrap().verified);
    }

    #[test]
    fn an_acknowledging_write_commits_with_its_acknowledgement_or_not_at_all() {
        let conn = fixture::identity_conn(&OWNER);
        ensure(&conn, &OWNER, &OLD).unwrap();
        mark_verified(&conn, &OWNER, &OLD, 1).unwrap();
        rotate(&conn, &OWNER, &OLD, &NEW, 9).unwrap();

        let failed: rusqlite::Result<()> = acknowledging(&conn, &OWNER, &NEW, |_| {
            Err(rusqlite::Error::QueryReturnedNoRows)
        });
        assert!(failed.is_err());
        assert!(
            verified_key_changed(&conn, &OWNER, &NEW).unwrap().is_some(),
            "a failed write acknowledges nothing"
        );

        assert_eq!(acknowledging(&conn, &OWNER, &NEW, |_| Ok(7)).unwrap(), 7);
        assert_eq!(verified_key_changed(&conn, &OWNER, &NEW).unwrap(), None);
        assert!(!get(&conn, &OWNER, &NEW).unwrap().unwrap().verified);
    }

    #[test]
    fn a_second_rotation_keeps_the_last_verified_key_and_verifying_acknowledges() {
        let conn = fixture::identity_conn(&OWNER);
        let newer = [4u8; 32];
        ensure(&conn, &OWNER, &OLD).unwrap();
        mark_verified(&conn, &OWNER, &OLD, 1).unwrap();
        rotate(&conn, &OWNER, &OLD, &NEW, 9).unwrap();
        rotate(&conn, &OWNER, &NEW, &newer, 12).unwrap();
        let row = get(&conn, &OWNER, &newer).unwrap().unwrap();
        assert_eq!((row.changed_from_key, row.changed_at), (Some(OLD), Some(9)));

        mark_verified(&conn, &OWNER, &newer, 20).unwrap();
        assert_eq!(verified_key_changed(&conn, &OWNER, &newer).unwrap(), None);
        assert!(get(&conn, &OWNER, &newer).unwrap().unwrap().verified);
    }
}
