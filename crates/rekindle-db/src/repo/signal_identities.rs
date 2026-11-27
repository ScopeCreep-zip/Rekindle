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

use crate::row::{query_opt, FromRow};
use rekindle_crypto::SafetyNumber;
use rekindle_types::intent::Escalation;
use rusqlite::{params, Connection};

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

impl FromRow for SignalIdentity {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(SignalIdentity {
            identity_key: row.get(0)?,
            verified: row.get(1)?,
            verified_at: row.get(2)?,
            changed_from_key: row.get(3)?,
            changed_at: row.get(4)?,
        })
    }
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
    query_opt(
        conn,
        "SELECT identity_key, verified, verified_at, changed_from_key, changed_at \
         FROM signal_identities WHERE owner_key = ?1 AND peer_identity = ?2",
        params![identity_text(owner), identity_text(peer)],
    )
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
/// the user last verified. Runs in the caller's transaction
/// (`repo::peer_rekey::rotate_peer`), so the friend row, the history and
/// this row move together.
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
    let previous = get(conn, owner, old)?;
    let (changed_from, changed_at) = match &previous {
        Some(row) if row.changed_from_key.is_some() => (row.changed_from_key, row.changed_at),
        Some(row) if row.verified => (Some(*old), Some(now_ms)),
        _ => (None, None),
    };
    conn.execute(
        "DELETE FROM signal_identities WHERE owner_key = ?1 AND peer_identity = ?2",
        params![identity_text(owner), identity_text(old)],
    )?;
    conn.execute(
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
    Ok(())
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
#[path = "signal_identities_tests.rs"]
mod tests;
