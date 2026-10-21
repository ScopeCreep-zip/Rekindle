//! `group_dm_invites` (plan E2.6): a creator's verified invite we have not
//! answered yet. It is held for at most [`INVITE_MAX_AGE`] —
//! `DELIVERY_HORIZON` (D24, Q2b), past which its creator's group has moved
//! on without us — and taken (deleted) on accept or decline. The owner of
//! its behaviour is `rekindle-dm` (`GroupDmDeps::{stash_invite,
//! take_invite}`); `invite` is the encoded `GroupDmInvite` body.

use std::time::Duration;

use rekindle_crypto::horizon::DELIVERY_HORIZON;
use rusqlite::{params, Connection, OptionalExtension as _};

/// How long an unanswered invite is held.
pub const INVITE_MAX_AGE: Duration = DELIVERY_HORIZON;

fn max_age_ms() -> i64 {
    i64::try_from(INVITE_MAX_AGE.as_millis()).unwrap_or(i64::MAX)
}

/// A held invite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteRow {
    pub group_id: String,
    /// The creator, whose session it came over.
    pub from_key: String,
    /// The encoded `GroupDmInvite` body.
    pub invite: Vec<u8>,
    /// Milliseconds.
    pub received_at: i64,
}

/// Hold an invite (a resend of it replaces the held one).
///
/// # Errors
/// The write failed (a column is refused).
pub fn put(conn: &Connection, owner_key: &str, row: &InviteRow) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO group_dm_invites (owner_key, group_id, from_key, invite, received_at) \
         VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(owner_key, group_id) DO UPDATE SET \
         from_key = excluded.from_key, invite = excluded.invite, received_at = excluded.received_at",
        params![owner_key, row.group_id, row.from_key, row.invite, row.received_at],
    )?;
    Ok(())
}

/// Take (delete and return) the invite to `group_id`, when it was received
/// within [`INVITE_MAX_AGE`] of `now_ms`; an expired one is deleted and
/// `None`.
///
/// # Errors
/// The query failed or the row is corrupt.
pub fn take(
    conn: &Connection,
    owner_key: &str,
    group_id: &str,
    now_ms: i64,
) -> rusqlite::Result<Option<InviteRow>> {
    let row = conn
        .query_row(
            "DELETE FROM group_dm_invites WHERE owner_key = ?1 AND group_id = ?2 \
             RETURNING group_id, from_key, invite, received_at",
            params![owner_key, group_id],
            |r| {
                Ok(InviteRow {
                    group_id: crate::row::get(r, "group_id")?,
                    from_key: crate::row::get(r, "from_key")?,
                    invite: crate::row::get(r, "invite")?,
                    received_at: crate::row::get(r, "received_at")?,
                })
            },
        )
        .optional()?;
    Ok(row.filter(|r| now_ms.saturating_sub(r.received_at) <= max_age_ms()))
}

/// Every invite received within [`INVITE_MAX_AGE`] of `now_ms`, newest
/// first.
///
/// # Errors
/// The query failed or a row is corrupt.
pub fn list(conn: &Connection, owner_key: &str, now_ms: i64) -> rusqlite::Result<Vec<InviteRow>> {
    let mut stmt = conn.prepare(
        "SELECT group_id, from_key, invite, received_at FROM group_dm_invites \
         WHERE owner_key = ?1 AND received_at >= ?2 ORDER BY received_at DESC",
    )?;
    let rows = stmt.query_map(
        params![owner_key, now_ms.saturating_sub(max_age_ms())],
        |r| {
            Ok(InviteRow {
                group_id: crate::row::get(r, "group_id")?,
                from_key: crate::row::get(r, "from_key")?,
                invite: crate::row::get(r, "invite")?,
                received_at: crate::row::get(r, "received_at")?,
            })
        },
    )?;
    rows.collect()
}

/// Delete every invite older than [`INVITE_MAX_AGE`] at `now_ms`.
///
/// # Errors
/// The write failed.
pub fn prune(conn: &Connection, owner_key: &str, now_ms: i64) -> rusqlite::Result<usize> {
    conn.execute(
        "DELETE FROM group_dm_invites WHERE owner_key = ?1 AND received_at < ?2",
        params![owner_key, now_ms.saturating_sub(max_age_ms())],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::fixture::{self, OWNER};

    const G: &str = "0123456789abcdef0123456789abcdef";

    fn row(at: i64) -> InviteRow {
        InviteRow {
            group_id: G.to_string(),
            from_key: "a".repeat(64),
            invite: vec![1, 2, 3],
            received_at: at,
        }
    }

    #[test]
    fn an_invite_is_held_replaced_and_taken_once() {
        let conn = fixture::conn();
        put(&conn, OWNER, &row(5)).unwrap();
        put(&conn, OWNER, &row(6)).unwrap();
        assert_eq!(take(&conn, OWNER, G, 7).unwrap(), Some(row(6)));
        assert_eq!(take(&conn, OWNER, G, 7).unwrap(), None, "taken once");
        let mut bad = row(5);
        bad.from_key = "short".into();
        assert!(put(&conn, OWNER, &bad).is_err());
    }

    #[test]
    fn an_invite_past_the_horizon_is_gone() {
        let conn = fixture::conn();
        let late = max_age_ms() + 10;
        put(&conn, OWNER, &row(5)).unwrap();
        assert_eq!(take(&conn, OWNER, G, late).unwrap(), None);
        put(&conn, OWNER, &row(5)).unwrap();
        assert_eq!(prune(&conn, OWNER, late).unwrap(), 1);
        put(&conn, OWNER, &row(5)).unwrap();
        assert_eq!(prune(&conn, OWNER, 6).unwrap(), 0);
    }
}
