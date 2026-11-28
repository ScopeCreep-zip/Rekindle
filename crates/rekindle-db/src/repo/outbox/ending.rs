//! A friend change that ends (or supersedes) a friendship settles the
//! streams it ends inside its own transaction (review RV5 item 3, D22):
//! each row is projected `failed` onto its echo and leaves the outbox in
//! the same commit as the change, so no crash between the two can leave a
//! row live for a friendship that ended. The caller publishes the echoes
//! once it committed.

use rekindle_types::message::SendState;
use rusqlite::{params, Connection};

use super::{prefix_rows, project, Row};

/// Which of a peer's streams a change ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    /// Our `dm` stream to them (a removal, a reject, an expiry).
    Dms,
    /// Our `friend_handshake` stream to them (a new friendship supersedes
    /// the rows framed under the one that ended).
    Handshakes,
    /// Both (a cancel, a block, their removal of us).
    Both,
}

impl Ended {
    fn kinds(self) -> &'static [&'static str] {
        match self {
            Self::Dms => &["dm"],
            Self::Handshakes => &["friend_handshake"],
            Self::Both => &["dm", "friend_handshake"],
        }
    }
}

/// Settle every row of the `ended` streams to `peer` `failed`: each is
/// projected onto its echo and deleted, in the caller's transaction (or as
/// one statement each when there is none). Returns the rows, as they were,
/// for the caller's post-commit echo.
///
/// # Errors
/// A read or write failed.
pub fn fail_to_peer(
    conn: &Connection,
    owner_key: &str,
    peer: &str,
    ended: Ended,
) -> rusqlite::Result<Vec<Row>> {
    let mut failed = Vec::new();
    for kind in ended.kinds() {
        let stream = format!("{kind}:{peer}");
        for row in prefix_rows(conn, owner_key, &stream)? {
            if row.stream != stream {
                continue;
            }
            project(conn, owner_key, &row.client_id, SendState::Failed)?;
            conn.execute(
                "DELETE FROM outbox WHERE owner_key = ?1 AND client_id = ?2",
                params![owner_key, row.client_id],
            )?;
            failed.push(row);
        }
    }
    Ok(failed)
}

/// When a friend-list row is queued (the clock's now, ms), when it is
/// first due, and the deadline it is attempted until (now +
/// `DELIVERY_HORIZON`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListWrite {
    pub at: i64,
    pub due: i64,
    pub deadline: i64,
}

/// The friend list's stream (`dht_write` kind, `friend_list` scope).
pub const FRIEND_LIST_STREAM: &str = "dht_write:friend_list";

/// Queue the friend list's `dht_write` row, due at `list.due`, as its stream's
/// only row (review RV5 item 5): in the caller's transaction, so the
/// change it follows and the row commit together. It has no payload; its
/// attempt writes the list as it is then.
///
/// # Errors
/// A write failed.
pub fn queue_friend_list(
    conn: &Connection,
    owner_key: &str,
    list: ListWrite,
) -> rusqlite::Result<i64> {
    let client_id = format!("friend_list@{}", list.at);
    supersede(
        conn,
        &super::NewRow {
            owner_key,
            stream: FRIEND_LIST_STREAM,
            client_id: &client_id,
            kind: "dht_write",
            payload: &[],
            next_at: list.due,
            deadline: list.deadline,
            depends_on: None,
        },
    )
}

/// Store `row` as its stream's only row: every earlier row of the stream
/// goes, unsent or failed at its deadline (a later friend-list write
/// supersedes the earlier). Run it in the caller's transaction (the
/// delete and the insert are two statements). Returns the row's
/// `stream_seq`.
///
/// # Errors
/// A write failed.
pub fn supersede(conn: &Connection, row: &super::NewRow<'_>) -> rusqlite::Result<i64> {
    conn.execute(
        "DELETE FROM outbox WHERE owner_key = ?1 AND stream = ?2",
        params![row.owner_key, row.stream],
    )?;
    super::append(conn, row, None)
}
