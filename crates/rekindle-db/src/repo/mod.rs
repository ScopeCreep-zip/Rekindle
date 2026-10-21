//! Repositories: every query on the tables more than one host reads or
//! writes (plan C5.3, `steps-10-19.md` N1).
//!
//! Each function takes the connection and runs on the database thread,
//! inside a [`crate::Db::call`] closure, so several can share one
//! transaction (`rusqlite::Transaction` derefs to the connection). This is
//! the shape matrix-sdk-sqlite gives its table helpers
//! (`SqliteKeyValueStoreConnExt`, `SqliteTransactionExt` over
//! `rusqlite::Connection`). The SQL lives here and nowhere else.

pub mod audit;
pub mod blocked;
pub mod chain_entries;
pub mod channels;
pub mod communities;
pub mod friend_groups;
pub mod friends;
pub mod governance_cache;
pub mod group_dm_invites;
pub mod group_dms;
pub mod identity;
pub mod intent_facts;
pub mod members;
pub mod messages;
pub mod outbox;
pub mod outgoing_invites;
pub mod peer_rekey;
pub mod pending_requests;
pub mod preferences;
pub mod presence_keys;
pub mod retry_requests;
mod role_ids;
pub mod send_log;
pub mod session_reset_requests;
pub mod signal_identities;
pub mod stream_cursors;
pub mod unprocessed;

#[cfg(test)]
mod constraints_tests;

/// An identity key as the per-owner tables store it: lowercase hex, the
/// form `identity.public_key` and `friends.public_key` hold.
pub(crate) fn identity_text(key: &[u8; 32]) -> String {
    hex::encode(key)
}

/// [`identity_text`] read back; anything else is a corrupt row naming
/// column `index`.
pub(crate) fn identity_from_text(text: &str, index: usize) -> rusqlite::Result<[u8; 32]> {
    let mut key = [0u8; 32];
    hex::decode_to_slice(text, &mut key).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            format!("identity key {text:?}: {e}").into(),
        )
    })?;
    Ok(key)
}

#[cfg(test)]
pub(crate) mod fixture {
    use rusqlite::Connection;

    /// The owner every fixture row belongs to.
    pub const OWNER: &str = "owner-pk";

    /// An in-memory database with the schema and [`OWNER`]'s identity row,
    /// which the per-owner tables reference.
    pub fn conn() -> Connection {
        crate::test_support::schema_conn(OWNER)
    }

    /// An in-memory database with the schema and an identity row for the
    /// key `owner` (the ratchet tables key their owner by its hex).
    pub fn identity_conn(owner: &[u8; 32]) -> Connection {
        crate::test_support::schema_conn(&super::identity_text(owner))
    }

    /// A `communities` row for [`OWNER`], which the per-community tables
    /// reference.
    pub fn community(conn: &Connection, id: &str) {
        conn.execute(
            "INSERT INTO communities (owner_key, id, name, joined_at) VALUES (?1, ?2, ?2, 0)",
            [OWNER, id],
        )
        .unwrap();
    }
}
