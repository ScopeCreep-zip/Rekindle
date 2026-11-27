//! `friends`: one row per (owner, friend).
//!
//! `friendship_state` holds the stored form of
//! [`FriendshipState`]; a value it does not name is a read error.

use crate::row::{query_list, query_opt, FromRow};
use rekindle_types::friend_store::FriendshipState;
use rusqlite::{params, Connection, OptionalExtension as _};

mod changes;
mod listing;
mod nickname;
mod participant;
mod presence;
mod records;
mod reply;
pub use changes::{
    accept_request, accept_with_records, add_by_code, block, cancel_request, changing_list,
    expire_pending_out, forget, forget_removed, mark_removing, reject_request,
    replace_with_invited, unfriended, RecordKeys,
};
pub use listing::{added_at, open_handshake_or_friend};
pub(crate) use nickname::rekey as rekey_nickname;
pub use nickname::{for_previous_key, set_nickname, PREVIOUS_KEY_SUFFIX};
pub use participant::{
    participant_record_key, peer_participant_record_key, set_participant_record_key,
    set_peer_participant_record_key,
};
pub use presence::{clear_presence_key, presence_keys, set_presence_key};
pub use records::{active_records, record, record_by_profile_key, records};
pub use reply::{
    is_accepted_row, reply_inbox_of, reply_inboxes, set_peer_reply, set_reply_inbox,
    take_peer_reply,
};

/// A friend as the buddy list loads it, with its group's name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FriendRow {
    pub public_key: String,
    pub display_name: String,
    pub nickname: Option<String>,
    /// The id of their group, when they are in one of ours.
    pub group_id: Option<i64>,
    pub group_name: Option<String>,
    pub dht_record_key: Option<String>,
    pub last_seen_at: Option<i64>,
    pub mailbox_dht_key: Option<String>,
    pub friendship_state: FriendshipState,
}

impl FromRow for FriendRow {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(FriendRow {
            public_key: row.get(0)?,
            display_name: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
            nickname: row.get(2)?,
            group_id: row.get(8)?,
            group_name: row.get(3)?,
            dht_record_key: row.get(4)?,
            last_seen_at: row.get(5)?,
            mailbox_dht_key: row.get(6)?,
            friendship_state: crate::row::get_enum(row, "friendship_state")?,
        })
    }
}

/// Every friend of `owner_key`.
///
/// # Errors
/// The query failed.
pub fn load_all(conn: &Connection, owner_key: &str) -> rusqlite::Result<Vec<FriendRow>> {
    query_list(
        conn,
        &format!(
            "SELECT f.public_key, f.display_name, {}, g.name, f.dht_record_key, \
         f.last_seen_at, f.mailbox_dht_key, f.friendship_state, f.group_id \
         FROM friends f LEFT JOIN friend_groups g ON f.group_id = g.id \
         WHERE f.owner_key = ?1",
            nickname::NICKNAME
        ),
        params![owner_key],
    )
}

/// Whether our request to `public_key` is outstanding (a `pending_out`
/// row): a request of theirs arriving now crosses ours (plan E2.5).
///
/// # Errors
/// The read failed.
pub fn is_pending_out(
    conn: &Connection,
    owner_key: &str,
    public_key: &str,
) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT 1 FROM friends WHERE owner_key = ?1 AND public_key = ?2 \
           AND friendship_state = 'pending_out'",
        params![owner_key, public_key],
        |_| Ok(()),
    )
    .optional()
    .map(|row| row.is_some())
}

/// Record an outgoing request: a `pending_out` row, unless the friend is
/// already there (a friend we are removing is added afresh).
///
/// # Errors
/// The write failed.
pub fn insert_pending_out(
    conn: &Connection,
    owner_key: &str,
    public_key: &str,
    display_name: &str,
    added_at: i64,
) -> rusqlite::Result<Vec<super::outbox::Row>> {
    changes::atomically(conn, |conn| {
        let ended = changes::end_removal(conn, owner_key, public_key)?;
        conn.execute(
            "INSERT OR IGNORE INTO friends (owner_key, public_key, display_name, added_at, friendship_state) \
             VALUES (?1, ?2, ?3, ?4, 'pending_out')",
            params![owner_key, public_key, display_name, added_at],
        )?;
        Ok(ended)
    })
}

/// Record an accepted friend, unless the friend is already there (a friend
/// we are removing is added afresh).
///
/// # Errors
/// The write failed.
pub fn insert_accepted(
    conn: &Connection,
    owner_key: &str,
    public_key: &str,
    display_name: &str,
    added_at: i64,
) -> rusqlite::Result<Vec<super::outbox::Row>> {
    changes::atomically(conn, |conn| {
        let ended = changes::end_removal(conn, owner_key, public_key)?;
        conn.execute(
            "INSERT OR IGNORE INTO friends (owner_key, public_key, display_name, added_at, friendship_state) \
             VALUES (?1, ?2, ?3, ?4, 'accepted')",
            params![owner_key, public_key, display_name, added_at],
        )?;
        Ok(ended)
    })
}

/// [`replace_with_invited`]'s rows, without its transaction.
fn replace_with_invited_rows(
    conn: &Connection,
    owner_key: &str,
    public_key: &str,
    display_name: &str,
    added_at: i64,
    profile_key: &str,
    mailbox_key: &str,
) -> rusqlite::Result<Vec<super::outbox::Row>> {
    let ended = changes::end_removal(conn, owner_key, public_key)?;
    delete(conn, owner_key, public_key)?;
    conn.execute(
        "INSERT INTO friends (owner_key, public_key, display_name, added_at, dht_record_key, \
         mailbox_dht_key, friendship_state) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending_out')",
        params![
            owner_key,
            public_key,
            display_name,
            added_at,
            profile_key,
            mailbox_key
        ],
    )?;
    Ok(ended)
}

/// Delete the friend's row.
///
/// # Errors
/// The write failed.
pub fn delete(conn: &Connection, owner_key: &str, public_key: &str) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM friends WHERE owner_key = ?1 AND public_key = ?2",
        params![owner_key, public_key],
    )?;
    // The friendship's cursor on their dm chain ends with it: the next
    // friendship's chain starts at seq 1, and its first hints step a fresh
    // cursor (live A/B finding 73).
    crate::repo::stream_cursors::reset(conn, owner_key, "dm", public_key)
}

/// Delete the outgoing requests sent before `cutoff`; their keys, each
/// with the rows to them the expiry failed in the same transaction.
///
/// # Errors
/// The query or a delete failed.
pub fn delete_pending_out_before(
    conn: &Connection,
    owner_key: &str,
    cutoff: i64,
) -> rusqlite::Result<Vec<(String, Vec<super::outbox::Row>)>> {
    changes::atomically(conn, |conn| {
        let expired: Vec<String> = conn
            .prepare(
                "SELECT public_key FROM friends \
                 WHERE owner_key = ?1 AND friendship_state = 'pending_out' AND added_at < ?2",
            )?
            .query_map(params![owner_key, cutoff], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        expired
            .into_iter()
            .map(|public_key| {
                let failed = changes::cancel_request(conn, owner_key, &public_key)?;
                Ok((public_key, failed))
            })
            .collect()
    })
}

/// The friend's local nickname and display name, when the friend is
/// stored: what an intent review names them by (plan D3.3).
///
/// # Errors
/// The query failed.
pub fn names(
    conn: &Connection,
    owner_key: &str,
    public_key: &str,
) -> rusqlite::Result<Option<(Option<String>, Option<String>)>> {
    query_opt(
        conn,
        &format!(
            "SELECT {}, f.display_name FROM friends f WHERE f.owner_key = ?1 AND f.public_key = ?2",
            nickname::NICKNAME
        ),
        params![owner_key, public_key],
    )
}

/// The friend's cached avatar, if any.
///
/// # Errors
/// The query failed.
pub fn avatar(
    conn: &Connection,
    owner_key: &str,
    public_key: &str,
) -> rusqlite::Result<Option<Vec<u8>>> {
    Ok(conn
        .query_row(
            "SELECT avatar_webp FROM friends WHERE owner_key = ?1 AND public_key = ?2",
            params![owner_key, public_key],
            |row| row.get(0),
        )
        .optional()?
        .flatten())
}

/// A column [`set`] writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Column {
    DisplayName,
    DhtRecordKey,
    MailboxDhtKey,
    LastSeenAt,
    GroupId,
}

impl Column {
    fn name(self) -> &'static str {
        match self {
            Self::DisplayName => "display_name",
            Self::DhtRecordKey => "dht_record_key",
            Self::MailboxDhtKey => "mailbox_dht_key",
            Self::LastSeenAt => "last_seen_at",
            Self::GroupId => "group_id",
        }
    }
}

/// Set one column of the friend's row.
///
/// # Errors
/// The write failed.
pub fn set(
    conn: &Connection,
    owner_key: &str,
    public_key: &str,
    column: Column,
    value: impl rusqlite::ToSql,
) -> rusqlite::Result<()> {
    conn.execute(
        &format!(
            "UPDATE friends SET {} = ?1 WHERE owner_key = ?2 AND public_key = ?3",
            column.name()
        ),
        params![value, owner_key, public_key],
    )?;
    Ok(())
}

/// Set the friend's friendship state.
///
/// # Errors
/// The write failed.
pub fn set_state(
    conn: &Connection,
    owner_key: &str,
    public_key: &str,
    state: FriendshipState,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE friends SET friendship_state = ?1 WHERE owner_key = ?2 AND public_key = ?3",
        params![state.as_str(), owner_key, public_key],
    )?;
    Ok(())
}

/// Set the profile and/or mailbox record keys; `None` keeps the stored one.
///
/// # Errors
/// The write failed.
pub fn set_record_keys(
    conn: &Connection,
    owner_key: &str,
    public_key: &str,
    profile_key: Option<&str>,
    mailbox_key: Option<&str>,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE friends SET dht_record_key = COALESCE(?1, dht_record_key), \
         mailbox_dht_key = COALESCE(?2, mailbox_dht_key) \
         WHERE owner_key = ?3 AND public_key = ?4",
        params![profile_key, mailbox_key, owner_key, public_key],
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "friends_tests.rs"]
mod tests;
