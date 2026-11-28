//! The creator-set round's rows (plan E3.2 S7s.3, decision (20)):
//! `creator_invites`, a friend's co-creator invite we have not answered
//! (taken on accept, decline or the creator's withdrawal), and
//! `pending_creations`, the co-creators a reserved creation of ours still
//! waits on (one removed when it answers or we remove it; every one taken
//! by the unlock sweep, which withdraws them). No secret is stored: the
//! anchor's one-time key lives only in memory (ADR 0011).

use rusqlite::{params, Connection, OptionalExtension as _};

/// A held co-creator invite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatorInviteRow {
    pub anchor: String,
    /// The creator, whose session it came over.
    pub inviter_key: String,
    pub community_name: String,
    /// Milliseconds.
    pub received_at: i64,
}

fn invite_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<CreatorInviteRow> {
    Ok(CreatorInviteRow {
        anchor: crate::row::get(r, "anchor")?,
        inviter_key: crate::row::get(r, "inviter_key")?,
        community_name: crate::row::get(r, "community_name")?,
        received_at: crate::row::get(r, "received_at")?,
    })
}

/// Hold an invite unless its inviter withdrew it. Whether it was stored.
///
/// # Errors
/// The write failed (a column is refused).
pub fn put_invite(
    conn: &Connection,
    owner_key: &str,
    row: &CreatorInviteRow,
) -> rusqlite::Result<bool> {
    Ok(conn.execute(
        "INSERT INTO creator_invites (owner_key, anchor, inviter_key, community_name, received_at) \
         SELECT ?1, ?2, ?3, ?4, ?5 WHERE NOT EXISTS (SELECT 1 FROM creator_invite_withdrawals \
         WHERE owner_key = ?1 AND anchor = ?2 AND inviter_key = ?3) \
         ON CONFLICT(owner_key, anchor) DO UPDATE SET received_at = excluded.received_at \
         WHERE creator_invites.inviter_key = excluded.inviter_key",
        params![owner_key, row.anchor, row.inviter_key, row.community_name, row.received_at],
    )? > 0)
}

/// Record a withdrawal and take any matching invite in one transaction.
///
/// # Errors
/// The transaction failed.
pub fn withdraw_invite(
    conn: &Connection,
    owner_key: &str,
    anchor: &str,
    inviter_key: &str,
) -> rusqlite::Result<bool> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO creator_invite_withdrawals (owner_key, anchor, inviter_key) \
         VALUES (?1, ?2, ?3) ON CONFLICT DO NOTHING",
        params![owner_key, anchor, inviter_key],
    )?;
    let held = take_invite(&tx, owner_key, anchor, inviter_key)?.is_some();
    tx.commit()?;
    Ok(held)
}

/// Take (delete and return) the invite to `anchor` from `inviter_key`; an
/// invite from anyone else is left alone.
///
/// # Errors
/// The query failed or the row is corrupt.
pub fn take_invite(
    conn: &Connection,
    owner_key: &str,
    anchor: &str,
    inviter_key: &str,
) -> rusqlite::Result<Option<CreatorInviteRow>> {
    conn.query_row(
        "DELETE FROM creator_invites WHERE owner_key = ?1 AND anchor = ?2 AND inviter_key = ?3 \
         RETURNING anchor, inviter_key, community_name, received_at",
        params![owner_key, anchor, inviter_key],
        invite_row,
    )
    .optional()
}

/// We accepted the held invite to `anchor` from `inviter_key`: it stays
/// held (accepted) until the creator's `CreatorJoin` or withdrawal takes
/// it. Whether such an invite was held.
///
/// # Errors
/// The write failed.
pub fn mark_accepted(
    conn: &Connection,
    owner_key: &str,
    anchor: &str,
    inviter_key: &str,
) -> rusqlite::Result<bool> {
    Ok(conn.execute(
        "UPDATE creator_invites SET accepted = 1 WHERE owner_key = ?1 AND anchor = ?2 \
         AND inviter_key = ?3",
        params![owner_key, anchor, inviter_key],
    )? > 0)
}

/// Take the accepted invite to `anchor` from `inviter_key` (its join link
/// arrived); an unaccepted one is left alone.
///
/// # Errors
/// The query failed or the row is corrupt.
pub fn take_accepted(
    conn: &Connection,
    owner_key: &str,
    anchor: &str,
    inviter_key: &str,
) -> rusqlite::Result<Option<CreatorInviteRow>> {
    conn.query_row(
        "DELETE FROM creator_invites WHERE owner_key = ?1 AND anchor = ?2 AND inviter_key = ?3 \
         AND accepted = 1 RETURNING anchor, inviter_key, community_name, received_at",
        params![owner_key, anchor, inviter_key],
        invite_row,
    )
    .optional()
}

/// The held invite to `anchor`, any inviter.
///
/// # Errors
/// The query failed or the row is corrupt.
pub fn invite(
    conn: &Connection,
    owner_key: &str,
    anchor: &str,
) -> rusqlite::Result<Option<CreatorInviteRow>> {
    conn.query_row(
        "SELECT anchor, inviter_key, community_name, received_at FROM creator_invites \
         WHERE owner_key = ?1 AND anchor = ?2",
        params![owner_key, anchor],
        invite_row,
    )
    .optional()
}

/// Every held invite, newest first.
///
/// # Errors
/// The query failed or a row is corrupt.
pub fn invites(conn: &Connection, owner_key: &str) -> rusqlite::Result<Vec<CreatorInviteRow>> {
    let mut stmt = conn.prepare(
        "SELECT anchor, inviter_key, community_name, received_at FROM creator_invites \
         WHERE owner_key = ?1 ORDER BY received_at DESC, anchor",
    )?;
    let rows = stmt.query_map(params![owner_key], invite_row)?;
    rows.collect()
}

/// Record that our reserved creation of `anchor` waits on `co_creators`.
///
/// # Errors
/// The write failed (a column is refused).
pub fn put_pending(
    conn: &Connection,
    owner_key: &str,
    anchor: &str,
    co_creators: &[String],
) -> rusqlite::Result<()> {
    for co_creator in co_creators {
        conn.execute(
            "INSERT OR IGNORE INTO pending_creations (owner_key, anchor, co_creator) \
             VALUES (?1, ?2, ?3)",
            params![owner_key, anchor, co_creator],
        )?;
    }
    Ok(())
}

/// Our creation of `anchor` no longer waits on `co_creator` (it answered,
/// or we removed it); whether it did.
///
/// # Errors
/// The write failed.
pub fn remove_pending(
    conn: &Connection,
    owner_key: &str,
    anchor: &str,
    co_creator: &str,
) -> rusqlite::Result<bool> {
    Ok(conn.execute(
        "DELETE FROM pending_creations WHERE owner_key = ?1 AND anchor = ?2 AND co_creator = ?3",
        params![owner_key, anchor, co_creator],
    )? > 0)
}

/// `co_creator` answered our creation of `anchor`: its row stays (a
/// creation that dies before its genesis still withdraws it at the next
/// unlock, decision (20)) but no second answer counts. Whether the
/// creation waited on it.
///
/// # Errors
/// The write failed.
pub fn answer_pending(
    conn: &Connection,
    owner_key: &str,
    anchor: &str,
    co_creator: &str,
) -> rusqlite::Result<bool> {
    Ok(conn.execute(
        "UPDATE pending_creations SET answered = 1 WHERE owner_key = ?1 AND anchor = ?2 \
         AND co_creator = ?3 AND answered = 0",
        params![owner_key, anchor, co_creator],
    )? > 0)
}

/// Forget our creation of `anchor` (its genesis is written or it was
/// cancelled).
///
/// # Errors
/// The write failed.
pub fn clear_pending(conn: &Connection, owner_key: &str, anchor: &str) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM pending_creations WHERE owner_key = ?1 AND anchor = ?2",
        params![owner_key, anchor],
    )?;
    Ok(())
}

/// Take (delete and return) every `(anchor, co_creator)` row: the unlock
/// sweep's creations that died before their genesis.
///
/// # Errors
/// The query failed or a row is corrupt.
pub fn take_all_pending(
    conn: &Connection,
    owner_key: &str,
) -> rusqlite::Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare(
        "DELETE FROM pending_creations WHERE owner_key = ?1 RETURNING anchor, co_creator",
    )?;
    let rows = stmt.query_map(params![owner_key], |r| {
        Ok((
            crate::row::get(r, "anchor")?,
            crate::row::get(r, "co_creator")?,
        ))
    })?;
    let mut all: Vec<(String, String)> = rows.collect::<rusqlite::Result<_>>()?;
    all.sort();
    Ok(all)
}

#[cfg(test)]
#[path = "creator_round_tests.rs"]
mod tests;
