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

/// Hold an invite (a resend replaces the held one).
///
/// # Errors
/// The write failed (a column is refused).
pub fn put_invite(
    conn: &Connection,
    owner_key: &str,
    row: &CreatorInviteRow,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO creator_invites (owner_key, anchor, inviter_key, community_name, received_at) \
         VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(owner_key, anchor) DO UPDATE SET \
         inviter_key = excluded.inviter_key, community_name = excluded.community_name, \
         received_at = excluded.received_at",
        params![owner_key, row.anchor, row.inviter_key, row.community_name, row.received_at],
    )?;
    Ok(())
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
mod tests {
    use super::*;
    use crate::repo::fixture::{self, OWNER};

    fn invite_at(at: i64) -> CreatorInviteRow {
        CreatorInviteRow {
            anchor: "VLD0:anchor".into(),
            inviter_key: "a".repeat(64),
            community_name: "LAN".into(),
            received_at: at,
        }
    }

    /// Decision (20): an invite survives until taken, only by its inviter.
    #[test]
    fn an_invite_is_held_and_taken_only_by_its_inviter() {
        let conn = fixture::conn();
        put_invite(&conn, OWNER, &invite_at(5)).unwrap();
        put_invite(&conn, OWNER, &invite_at(6)).unwrap();
        assert_eq!(invites(&conn, OWNER).unwrap(), vec![invite_at(6)]);
        assert_eq!(
            take_invite(&conn, OWNER, "VLD0:anchor", &"b".repeat(64)).unwrap(),
            None
        );
        assert_eq!(
            take_invite(&conn, OWNER, "VLD0:anchor", &"a".repeat(64)).unwrap(),
            Some(invite_at(6))
        );
        assert!(invites(&conn, OWNER).unwrap().is_empty());
        put_invite(&conn, OWNER, &invite_at(7)).unwrap();
        let a = "a".repeat(64);
        assert_eq!(
            take_accepted(&conn, OWNER, "VLD0:anchor", &a).unwrap(),
            None,
            "not accepted"
        );
        assert!(mark_accepted(&conn, OWNER, "VLD0:anchor", &a).unwrap());
        assert_eq!(
            take_accepted(&conn, OWNER, "VLD0:anchor", &a).unwrap(),
            Some(invite_at(7))
        );
        let mut bad = invite_at(5);
        bad.inviter_key = "short".into();
        assert!(put_invite(&conn, OWNER, &bad).is_err());
    }

    /// Decision (20): the sweep takes every waiting co-creator once.
    #[test]
    fn the_sweep_takes_every_pending_co_creator_once() {
        let conn = fixture::conn();
        let (b, c) = ("b".repeat(64), "c".repeat(64));
        put_pending(&conn, OWNER, "VLD0:x", &[b.clone(), c.clone()]).unwrap();
        assert!(remove_pending(&conn, OWNER, "VLD0:x", &c).unwrap());
        assert!(!remove_pending(&conn, OWNER, "VLD0:x", &c).unwrap());
        put_pending(&conn, OWNER, "VLD0:y", std::slice::from_ref(&c)).unwrap();
        clear_pending(&conn, OWNER, "VLD0:y").unwrap();
        assert_eq!(
            take_all_pending(&conn, OWNER).unwrap(),
            vec![("VLD0:x".to_string(), b)]
        );
        assert!(take_all_pending(&conn, OWNER).unwrap().is_empty());
    }
}
