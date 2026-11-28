//! Friend changes that touch several rows, each applied whole or not at
//! all (plan audit D-1): one transaction per change, or the caller's when
//! it is already inside one.

use rekindle_types::friend_store::FriendshipState;
use rusqlite::{params, Connection, OptionalExtension as _};

use super::{
    delete, insert_accepted, insert_pending_out, replace_with_invited_rows, set_record_keys,
    set_state,
};
use crate::repo::outbox::{self, fail_to_peer, Ended, Row};
use crate::repo::{blocked, pending_requests};

/// Run `change` atomically: in a transaction of its own, or in the
/// caller's when `conn` is already inside one (a `Transaction` derefs to
/// the connection).
pub(crate) fn atomically<T>(
    conn: &Connection,
    change: impl FnOnce(&Connection) -> rusqlite::Result<T>,
) -> rusqlite::Result<T> {
    if !conn.is_autocommit() {
        return change(conn);
    }
    let tx = conn.unchecked_transaction()?;
    let value = change(&tx)?;
    tx.commit()?;
    Ok(value)
}

/// A new friendship with `public_key` starts (a request, an invite, an
/// accept): every handshake row to them framed under an ended one settles
/// `failed` in this transaction (an `Unfriended` awaiting its ack, an old
/// `FriendsKey`; live A/B finding 38) and is returned for the echo; a
/// friend still pending or accepted keeps every row (that friendship goes
/// on).
///
/// A friend we are removing who is added again ends the removal here: its
/// row goes, and so does what is still queued to them (the `Unfriended`
/// notice must not follow the new request out). The sessions with them
/// stay: the new friendship's accept replaces the active one (plan E2.5h,
/// `Reinitiation::Befriended`).
pub(super) fn end_removal(
    conn: &Connection,
    owner_key: &str,
    public_key: &str,
) -> rusqlite::Result<Vec<Row>> {
    let stored: Option<String> = conn
        .query_row(
            "SELECT friendship_state FROM friends WHERE owner_key = ?1 AND public_key = ?2",
            params![owner_key, public_key],
            |row| row.get(0),
        )
        .optional()?;
    // A friendship that is still starting or open keeps every row it
    // committed: only one that ended (no row left, or one we are removing)
    // has rows to fail.
    let ended = match stored.as_deref() {
        None | Some("removing") => fail_to_peer(conn, owner_key, public_key, Ended::Handshakes)?,
        Some(_) => return Ok(Vec::new()),
    };
    if stored.is_some() {
        outbox::delete_to_peer(conn, owner_key, public_key)?;
        delete(conn, owner_key, public_key)?;
    }
    Ok(ended)
}

/// Run `change`, a change of our friend list, and queue the list's
/// `dht_write` row in the same transaction (review RV5 item 5, D22: the
/// row is written with the change), so no crash between the two loses
/// the change's write.
///
/// # Errors
/// The change or the row failed; nothing is kept.
pub fn changing_list<T>(
    conn: &Connection,
    owner_key: &str,
    list: outbox::ListWrite,
    change: impl FnOnce(&Connection) -> rusqlite::Result<T>,
) -> rusqlite::Result<T> {
    atomically(conn, |conn| {
        let value = change(conn)?;
        outbox::queue_friend_list(conn, owner_key, list)?;
        Ok(value)
    })
}

/// [`super::delete_pending_out_before`], queueing the friend list's row in
/// the same transaction when a request expired (the 3-minute tick that
/// expires nothing queues nothing).
///
/// # Errors
/// A read or write failed; nothing is kept.
pub fn expire_pending_out(
    conn: &Connection,
    owner_key: &str,
    cutoff: i64,
    list: outbox::ListWrite,
) -> rusqlite::Result<Vec<(String, Vec<Row>)>> {
    atomically(conn, |conn| {
        let expired = super::delete_pending_out_before(conn, owner_key, cutoff)?;
        if !expired.is_empty() {
            outbox::queue_friend_list(conn, owner_key, list)?;
        }
        Ok(expired)
    })
}

/// The record keys a friend names; `None` keeps the stored one.
#[derive(Debug, Clone, Copy, Default)]
pub struct RecordKeys<'a> {
    pub profile: Option<&'a str>,
    pub mailbox: Option<&'a str>,
}

/// We accept `public_key`'s request: the friend stored `accepted` with the
/// records it named, and the request deleted.
///
/// # Errors
/// A write failed; nothing is kept.
pub fn accept_request(
    conn: &Connection,
    owner_key: &str,
    public_key: &str,
    display_name: &str,
    added_at: i64,
    keys: RecordKeys<'_>,
) -> rusqlite::Result<Vec<Row>> {
    atomically(conn, |conn| {
        let ended = insert_accepted(conn, owner_key, public_key, display_name, added_at)?;
        pending_requests::delete(conn, owner_key, public_key)?;
        set_record_keys(conn, owner_key, public_key, keys.profile, keys.mailbox)?;
        Ok(ended)
    })
}

/// We ask `public_key` by their friend code: a `pending_out` row naming
/// the profile record the code gave, with the handshake rows of an ended
/// friendship failed ([`end_removal`]), in one transaction; those rows.
///
/// # Errors
/// A write failed; nothing is kept.
pub fn add_by_code(
    conn: &Connection,
    owner_key: &str,
    public_key: &str,
    display_name: &str,
    added_at: i64,
    profile_key: &str,
) -> rusqlite::Result<Vec<Row>> {
    atomically(conn, |conn| {
        let ended = insert_pending_out(conn, owner_key, public_key, display_name, added_at)?;
        set_record_keys(conn, owner_key, public_key, Some(profile_key), None)?;
        Ok(ended)
    })
}

/// We reject `public_key`'s request: it goes, and our pending messages to
/// them fail with it, in one transaction; those rows.
///
/// # Errors
/// A write failed; nothing is kept.
pub fn reject_request(
    conn: &Connection,
    owner_key: &str,
    public_key: &str,
) -> rusqlite::Result<Vec<Row>> {
    atomically(conn, |conn| {
        pending_requests::delete(conn, owner_key, public_key)?;
        fail_to_peer(conn, owner_key, public_key, Ended::Dms)
    })
}

/// Our pending friend `public_key` is accepted (their accept arrived, or
/// both of us asked): `accepted` with the records they named, the name
/// they gave when `Some`, and any request of theirs deleted.
///
/// # Errors
/// A write failed; nothing is kept.
pub fn accept_with_records(
    conn: &Connection,
    owner_key: &str,
    public_key: &str,
    display_name: Option<&str>,
    keys: RecordKeys<'_>,
) -> rusqlite::Result<()> {
    atomically(conn, |conn| {
        set_state(conn, owner_key, public_key, FriendshipState::Accepted)?;
        if let Some(name) = display_name {
            super::set(
                conn,
                owner_key,
                public_key,
                super::Column::DisplayName,
                Some(name.to_string()),
            )?;
        }
        set_record_keys(conn, owner_key, public_key, keys.profile, keys.mailbox)?;
        pending_requests::delete(conn, owner_key, public_key)
    })
}

/// Block `public_key`: the friend, their request, everything queued to
/// them (each row failed onto its echo) go, and the block is stored; the
/// failed rows. The sessions with them stay (plan E2.5h).
///
/// # Errors
/// A write failed; nothing is kept.
pub fn block(
    conn: &Connection,
    owner_key: &str,
    public_key: &str,
    display_name: &str,
    blocked_at: i64,
) -> rusqlite::Result<Vec<Row>> {
    atomically(conn, |conn| {
        // What the feed logged about them goes (Infoview design §3.3).
        crate::repo::activity::purge_friend(conn, owner_key, public_key)?;
        delete(conn, owner_key, public_key)?;
        pending_requests::delete(conn, owner_key, public_key)?;
        let failed = fail_to_peer(conn, owner_key, public_key, Ended::Both)?;
        blocked::insert(conn, owner_key, public_key, display_name, blocked_at)?;
        Ok(failed)
    })
}

/// We remove `public_key`: the row is `removing`, their request goes, and
/// the `Unfriended` notice is stored as `unfriended`, a row of their
/// `friend_handshake` stream (plan E2.5), and our pending messages and
/// handshake rows to them fail first, so the notice never waits behind a
/// row of the friendship it ends (a `FriendsKey` whose receipt is lost):
/// one transaction; the failed rows.
///
/// # Errors
/// A write failed; nothing is kept.
pub fn mark_removing(
    conn: &Connection,
    owner_key: &str,
    public_key: &str,
    unfriended: &outbox::NewRow<'_>,
) -> rusqlite::Result<Vec<Row>> {
    atomically(conn, |conn| {
        set_state(conn, owner_key, public_key, FriendshipState::Removing)?;
        super::nickname::clear(conn, owner_key, public_key)?;
        super::clear_presence_key(conn, owner_key, public_key)?;
        // What they shared ends with the friendship (plan E7.2).
        crate::repo::bio::set_friend(conn, owner_key, public_key, None)?;
        // And what the feed logged about them (Infoview design §3.3).
        crate::repo::activity::purge_friend(conn, owner_key, public_key)?;
        conn.execute(
            "DELETE FROM friend_game_totals WHERE owner_key = ?1 AND friend_key = ?2",
            params![owner_key, public_key],
        )?;
        pending_requests::delete(conn, owner_key, public_key)?;
        let failed = fail_to_peer(conn, owner_key, public_key, Ended::Both)?;
        outbox::append(conn, unfriended, None)?;
        Ok(failed)
    })
}

/// Forget `public_key`: their row and their request, and our pending
/// messages to them fail; the failed rows.
///
/// # Errors
/// A write failed; nothing is kept.
pub fn forget(conn: &Connection, owner_key: &str, public_key: &str) -> rusqlite::Result<Vec<Row>> {
    atomically(conn, |conn| {
        delete(conn, owner_key, public_key)?;
        pending_requests::delete(conn, owner_key, public_key)?;
        fail_to_peer(conn, owner_key, public_key, Ended::Dms)
    })
}

/// Our request to `public_key` was cancelled: their row, our rows to them
/// (failed onto their echoes, returned) go. Any session with them stays
/// (plan E2.5h: sessions are erased only by eviction from the archive or
/// an identity reset).
///
/// # Errors
/// A write failed; nothing is kept.
pub fn cancel_request(
    conn: &Connection,
    owner_key: &str,
    public_key: &str,
) -> rusqlite::Result<Vec<Row>> {
    atomically(conn, |conn| {
        delete(conn, owner_key, public_key)?;
        super::nickname::clear(conn, owner_key, public_key)?;
        fail_to_peer(conn, owner_key, public_key, Ended::Both)
    })
}

/// `public_key` removed us: their row, their request, everything queued to
/// them (failed onto its echo, returned) go; the sessions with them stay
/// (plan E2.5h).
///
/// # Errors
/// A write failed; nothing is kept.
pub fn unfriended(
    conn: &Connection,
    owner_key: &str,
    public_key: &str,
) -> rusqlite::Result<Vec<Row>> {
    atomically(conn, |conn| {
        let mut failed = forget(conn, owner_key, public_key)?;
        super::nickname::clear(conn, owner_key, public_key)?;
        failed.extend(fail_to_peer(
            conn,
            owner_key,
            public_key,
            Ended::Handshakes,
        )?);
        Ok(failed)
    })
}

/// Forget the friends we removed once no send to them is unsettled (their
/// `Unfriended` row was acknowledged or failed at its deadline, plan
/// E2.5): their rows and the rest of their streams; their keys. The
/// sessions with them stay (plan E2.5h).
///
/// # Errors
/// The query or a delete failed; nothing is kept.
pub fn forget_removed(conn: &Connection, owner_key: &str) -> rusqlite::Result<Vec<String>> {
    atomically(conn, |conn| {
        let done = conn
            .prepare(&format!(
                "SELECT public_key FROM friends f \
                 WHERE owner_key = ?1 AND friendship_state = 'removing' \
                   AND NOT {}",
                outbox::UNSETTLED_TO_FRIEND
            ))?
            .query_map(params![owner_key], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        for public_key in &done {
            outbox::delete_to_peer(conn, owner_key, public_key)?;
            delete(conn, owner_key, public_key)?;
            super::nickname::clear(conn, owner_key, public_key)?;
        }
        Ok(done)
    })
}

/// Replace any row for the friend with a `pending_out` row carrying the
/// record keys an invite named; the handshake rows it ended ([`end_removal`]).
///
/// # Errors
/// A write failed; nothing is kept.
pub fn replace_with_invited(
    conn: &Connection,
    owner_key: &str,
    public_key: &str,
    display_name: &str,
    added_at: i64,
    profile_key: &str,
    mailbox_key: &str,
) -> rusqlite::Result<Vec<Row>> {
    atomically(conn, |conn| {
        replace_with_invited_rows(
            conn,
            owner_key,
            public_key,
            display_name,
            added_at,
            profile_key,
            mailbox_key,
        )
    })
}

#[cfg(test)]
mod tests;
