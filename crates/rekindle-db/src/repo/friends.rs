//! `friends`: one row per (owner, friend).
//!
//! `friendship_state` holds the stored form of
//! [`FriendshipState`]; a value it does not name is a read error.

use rekindle_types::friend_store::FriendshipState;
use rusqlite::{params, Connection, OptionalExtension as _};

mod changes;
mod listing;
mod nickname;
mod participant;
mod presence;
mod records;
pub use changes::{
    accept_request, accept_with_records, add_by_code, block, cancel_request, changing_list,
    expire_pending_out, forget, forget_removed, mark_removing, reject_request,
    replace_with_invited, unfriended, RecordKeys,
};
pub use listing::added_at;
pub(crate) use nickname::rekey as rekey_nickname;
pub use nickname::{for_previous_key, set_nickname, PREVIOUS_KEY_SUFFIX};
pub use participant::{
    participant_record_key, peer_participant_record_key, set_participant_record_key,
    set_peer_participant_record_key,
};
pub use presence::{clear_presence_key, presence_keys, set_presence_key};
pub use records::{active_records, record, record_by_profile_key, records};

/// A friend as the buddy list loads it, with its group's name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FriendRow {
    pub public_key: String,
    pub display_name: String,
    pub nickname: Option<String>,
    pub group_name: Option<String>,
    pub dht_record_key: Option<String>,
    pub last_seen_at: Option<i64>,
    pub mailbox_dht_key: Option<String>,
    pub friendship_state: FriendshipState,
}

/// Every friend of `owner_key`.
///
/// # Errors
/// The query failed.
pub fn load_all(conn: &Connection, owner_key: &str) -> rusqlite::Result<Vec<FriendRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT f.public_key, f.display_name, {}, g.name, f.dht_record_key, \
         f.last_seen_at, f.mailbox_dht_key, f.friendship_state \
         FROM friends f LEFT JOIN friend_groups g ON f.group_id = g.id \
         WHERE f.owner_key = ?1",
        nickname::NICKNAME
    ))?;
    let rows = stmt
        .query_map(params![owner_key], |row| {
            Ok(FriendRow {
                public_key: row.get(0)?,
                display_name: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                nickname: row.get(2)?,
                group_name: row.get(3)?,
                dht_record_key: row.get(4)?,
                last_seen_at: row.get(5)?,
                mailbox_dht_key: row.get(6)?,
                friendship_state: crate::row::get_enum(row, "friendship_state")?,
            })
        })?
        .collect();
    rows
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
    conn.query_row(
        &format!(
            "SELECT {}, f.display_name FROM friends f WHERE f.owner_key = ?1 AND f.public_key = ?2",
            nickname::NICKNAME
        ),
        params![owner_key, public_key],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
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
mod tests {
    use super::*;
    use crate::repo::fixture::{self, OWNER};

    #[test]
    fn an_outgoing_request_loads_as_pending_out_and_accept_does_not_overwrite_it() {
        let conn = fixture::conn();
        insert_pending_out(&conn, OWNER, "bob", "Bob", 1).unwrap();
        insert_accepted(&conn, OWNER, "bob", "Other", 2).unwrap();
        let rows = load_all(&conn, OWNER).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].display_name, "Bob");
        assert_eq!(rows[0].friendship_state, FriendshipState::PendingOut);
    }

    #[test]
    fn set_writes_one_column_and_record_keys_keep_the_stored_value_for_none() {
        let conn = fixture::conn();
        replace_with_invited(&conn, OWNER, "bob", "Bob", 1, "profile", "mailbox").unwrap();
        set_nickname(&conn, OWNER, "bob", Some("bobby")).unwrap();
        set_record_keys(&conn, OWNER, "bob", None, Some("mailbox-2")).unwrap();
        let row = &load_all(&conn, OWNER).unwrap()[0];
        assert_eq!(row.nickname.as_deref(), Some("bobby"));
        assert_eq!(row.dht_record_key.as_deref(), Some("profile"));
        assert_eq!(row.mailbox_dht_key.as_deref(), Some("mailbox-2"));
        let record = record_by_profile_key(&conn, OWNER, "profile")
            .unwrap()
            .unwrap();
        assert_eq!(record.status, FriendshipState::PendingOut);
    }

    #[test]
    fn a_removed_friend_is_forgotten_only_once_its_unfriended_row_settles() {
        use crate::repo::outbox;
        let conn = fixture::conn();
        insert_accepted(&conn, OWNER, "bob", "Bob", 1).unwrap();
        insert_accepted(&conn, OWNER, "bobby", "Bobby", 1).unwrap();
        set_state(&conn, OWNER, "bob", FriendshipState::Removing).unwrap();
        outbox::queue_for_test(
            &conn,
            OWNER,
            "friend_handshake:bob",
            "unfriended:bob",
            b"{}",
        );
        // A send to another peer whose key extends bob's holds nothing.
        outbox::queue_for_test(&conn, OWNER, "dm:bobby", "m1", b"{}");
        assert!(forget_removed(&conn, OWNER).unwrap().is_empty());
        assert_eq!(
            load_all(&conn, OWNER).unwrap()[0].friendship_state,
            FriendshipState::Removing
        );
        // Failed at its deadline: settled for the removal, and forgotten
        // with the friend.
        let failed = outbox::Update {
            state: rekindle_types::message::SendState::Failed,
            attempts: 3,
            next_at: 0,
            deadline: 0,
            wedge_reason: Some("deadline"),
        };
        outbox::update(&conn, OWNER, "unfriended:bob", &failed).unwrap();
        assert_eq!(forget_removed(&conn, OWNER).unwrap(), ["bob"]);
        assert_eq!(load_all(&conn, OWNER).unwrap().len(), 1);
        assert_eq!(
            outbox::unsettled_count(&conn, OWNER).unwrap(),
            1,
            "bobby's stays"
        );
    }

    #[test]
    fn adding_a_friend_we_are_removing_ends_the_removal() {
        let conn = fixture::conn();
        insert_accepted(&conn, OWNER, "bob", "Bob", 1).unwrap();
        set_state(&conn, OWNER, "bob", FriendshipState::Removing).unwrap();
        crate::repo::outbox::queue_for_test(
            &conn,
            OWNER,
            "friend_handshake:bob",
            "unfriended:bob",
            b"{}",
        );
        conn.execute(
            "INSERT INTO ratchet_sessions (owner_key, peer_identity, peer_device, record, updated_at) \
             VALUES (?1, 'bob', 'd', x'00', 1)",
            params![OWNER.to_ascii_lowercase()],
        )
        .unwrap();
        insert_pending_out(&conn, OWNER, "bob", "Bob", 2).unwrap();
        assert_eq!(
            load_all(&conn, OWNER).unwrap()[0].friendship_state,
            FriendshipState::PendingOut
        );
        assert_eq!(
            crate::repo::outbox::unsettled_count(&conn, OWNER).unwrap(),
            0
        );
        let sessions: i64 = conn
            .query_row("SELECT COUNT(*) FROM ratchet_sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(sessions, 1, "the removal's session stays (plan E2.5h)");
    }

    #[test]
    fn cancelling_a_request_keeps_a_session_from_an_ended_friendship() {
        let conn = fixture::conn();
        insert_pending_out(&conn, OWNER, "bob", "Bob", 1).unwrap();
        conn.execute(
            "INSERT INTO ratchet_sessions (owner_key, peer_identity, peer_device, record, updated_at) \
             VALUES (?1, 'bob', 'd', x'00', 1)",
            params![OWNER.to_ascii_lowercase()],
        )
        .unwrap();
        cancel_request(&conn, OWNER, "bob").unwrap();
        assert!(load_all(&conn, OWNER).unwrap().is_empty());
        let sessions: i64 = conn
            .query_row("SELECT COUNT(*) FROM ratchet_sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(sessions, 1);
    }

    #[test]
    fn a_new_request_keeps_a_session_from_an_ended_friendship() {
        let conn = fixture::conn();
        conn.execute(
            "INSERT INTO ratchet_sessions (owner_key, peer_identity, peer_device, record, updated_at) \
             VALUES (?1, 'bob', 'd', x'00', 1)",
            params![OWNER.to_ascii_lowercase()],
        )
        .unwrap();
        insert_pending_out(&conn, OWNER, "bob", "Bob", 1).unwrap();
        let sessions: i64 = conn
            .query_row("SELECT COUNT(*) FROM ratchet_sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(sessions, 1);
    }

    #[test]
    fn a_corrupt_friendship_state_is_a_read_error() {
        let conn = fixture::conn();
        insert_accepted(&conn, OWNER, "bob", "Bob", 1).unwrap();
        conn.pragma_update(None, "ignore_check_constraints", true)
            .unwrap();
        conn.execute("UPDATE friends SET friendship_state = 'active'", [])
            .unwrap();
        let err = load_all(&conn, OWNER).unwrap_err().to_string();
        assert!(err.contains("friendship_state"), "{err}");
        assert!(record(&conn, OWNER, "bob").is_err());
    }

    #[test]
    fn only_old_outgoing_requests_expire() {
        let conn = fixture::conn();
        insert_pending_out(&conn, OWNER, "old", "Old", 1).unwrap();
        insert_pending_out(&conn, OWNER, "new", "New", 100).unwrap();
        insert_accepted(&conn, OWNER, "friend", "Friend", 1).unwrap();
        assert_eq!(
            delete_pending_out_before(&conn, OWNER, 50)
                .unwrap()
                .into_iter()
                .map(|(key, _)| key)
                .collect::<Vec<_>>(),
            ["old"]
        );
        let mut left: Vec<String> = load_all(&conn, OWNER)
            .unwrap()
            .into_iter()
            .map(|r| r.public_key)
            .collect();
        left.sort();
        assert_eq!(left, ["friend", "new"]);
    }

    #[test]
    fn a_friend_without_an_avatar_has_none() {
        let conn = fixture::conn();
        insert_accepted(&conn, OWNER, "bob", "Bob", 1).unwrap();
        assert_eq!(avatar(&conn, OWNER, "bob").unwrap(), None);
        assert_eq!(avatar(&conn, OWNER, "ghost").unwrap(), None);
    }
}
