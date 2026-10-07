//! Opening the database: pragmas, the schema-version check, and the reset
//! when the schema changed.

use std::path::Path;

use rusqlite::Connection;

use crate::Db;

/// The schema. Edited in place before release; bump [`SCHEMA_VERSION`]
/// with every change (CLAUDE.md "No migration files").
pub const SCHEMA: &str = include_str!("../schema/001_init.sql");

/// Bump this every time `schema/001_init.sql` changes, or when a DHT record
/// layout is renumbered: the reset also wipes Veilid storage, so stale
/// records written under the old layout cannot be read back with the new
/// indices. On mismatch the database is dropped and recreated; the app is
/// not live yet.
///
/// 83: `dht_outbox`, the durable record writes a logout left unsent
/// (plan C7.6h); the next login of the same identity re-pushes them.
///
/// 81: the friend-list DHT record. `FriendEntry` gained `dm_log_key`
/// and `profileDhtKey`/`dmLogKey` joined `schemas/friend.capnp`, which
/// had only four fields while the Rust struct had five — the encoder
/// dropped `profile_dht_key` on every write and the decoder set it to
/// `None` with a comment saying so. The daemon track also wrote this
/// record in postcard while the desktop wrote Cap'n Proto, so a list
/// written by either is unreadable to the other.
///
/// 80: the account record's header shed the three child
/// `DHTShortArray` pointers and their owner keypairs. Creating an
/// account allocated a contact list, a chat list and an invitation list
/// that nothing ever wrote an entry to, and every login reopened all
/// three. `AccountHeader` is now the five fields it actually carries,
/// so a header written under the old schema decodes its text fields at
/// the wrong ordinals.
///
/// 79: the session membership record lost `is_operator`,
/// `community_mailbox_key` and `join_inbox_key`. All three were the
/// coordinator's, and the mailbox they named was never created by
/// anything — `create_community_mailbox` had no callers, so the route
/// the authority loop published into it went nowhere.
///
/// 76: `MemberPresence` gained `departed`, the signed tombstone a
/// leaver writes into its own registry slot so the slot can be reused.
/// Live rows are unaffected — the field is `skip_serializing_if`, so a
/// non-departed row stays byte-identical and old readers still verify
/// it. Only a departed row carries it, and an old reader drops the
/// unknown field before recomputing `signing_bytes()`, fails the
/// signature, and treats the slot as reclaimable too: both ends reach
/// the same answer. Bumped because the wire format grew a field, not
/// because the tracks disagree.
///
/// 75: the registry MEK vault is gone from community creation, and the
/// creator no longer writes itself into a shared member index. Both were
/// owner-subkey writes that `o_cnt: 0` grants nobody a credential for,
/// and `communities-channels.md` says the MEK is *never* written to DHT.
/// A community created under the old flow therefore carries a vault and
/// an index entry that new peers neither write nor read, while its
/// genesis MEK sits on the DHT where it does not belong. Rotation is now
/// peer-to-peer for both shells (`rekindle-mek-rotation`), delivered by
/// `app_call` rather than published.
///
/// 74: admission governance entries. `GovernanceEntry` gained
/// `JoinRequested`, `MemberApproved`, `MemberRejected` and
/// `AdmissionPolicy` (Cap'n Proto union arms 34-37), so a peer on the
/// old schema cannot read entries written by a new one — the union
/// discriminant is unknown to it. Also flips the daemon to the v2.0
/// self-sovereign join: members claim their own registry slot from the
/// invite's shared slot seed instead of being assigned one, so slots
/// written under the old flow used a locally-derived seed that no longer
/// resolves.
///
/// 73: profile DHT record renumbered. The two tracks had disagreed about
/// subkey 8 (Strand Relay pool vs friend-inbox key); the friend-inbox
/// pair moved to 9/10 and the record now allocates 11 subkeys. See
/// `rekindle_types::dht_layout::profile`.
/// 77: the member registry lost its last community-wide subkeys. The
/// member index (0), MEK vault (1) and moderation queue (5) were v1.0
/// owner subkeys that `o_cnt: 0` credentials nobody to write, and each
/// collided with a real member slot. Membership is now derived by
/// scanning signed presence rows, the MEK is delivered peer-to-peer and
/// never written to the DHT, and admission is a `GovernanceEntry`.
///
/// Landing with it: the daemon's channel storage moved from a
/// per-member DFLT `DhtLog` to the SMPL `(channel, segment)` records the
/// desktop already wrote, so the two tracks finally read and write the
/// same messages. Discovery is `ChannelCreated.record_key` plus
/// `ChannelSegmentLinked` out of merged governance, which is what made
/// the member index deletable.
/// 78: genesis entries gained `AdmissionPolicy` at lamport 1, shifting
/// the other five up by one. Every community created before this has an
/// entry set that cannot express `ApprovalRequired` — the variant was
/// merged, validated and Cap'n Proto encoded, and nothing ever wrote it,
/// so the mode was unreachable and every community was `Open`.
pub const SCHEMA_VERSION: i64 = 83;

/// Opening the database failed.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// SQLite refused an operation; `context` says which.
    #[error("{context}: {source}")]
    Sqlite {
        context: &'static str,
        source: rusqlite::Error,
    },
}

fn sqlite(context: &'static str) -> impl FnOnce(rusqlite::Error) -> DbError {
    move |source| DbError::Sqlite { context, source }
}

/// The opened database, and whether its schema was recreated.
pub struct DbOpenResult {
    pub db: Db,
    /// `true` when the stored schema version differed and every table was
    /// dropped and recreated. The caller wipes the storage that has to
    /// stay in step with it (vault files, Veilid storage).
    pub schema_reset: bool,
}

/// Open (or create) the plaintext database at `path`: WAL, foreign keys,
/// and the schema recreated when its version is not [`SCHEMA_VERSION`].
/// `":memory:"` opens an in-memory database.
///
/// # Errors
/// [`DbError`] naming the step that failed, including an unreadable
/// `user_version` (which used to be read as 0 and silently reset the
/// database).
pub fn open(path: &Path) -> Result<DbOpenResult, DbError> {
    let conn = Connection::open(path).map_err(sqlite("open database"))?;
    conn.execute_batch("PRAGMA journal_mode=WAL;")
        .map_err(sqlite("set WAL mode"))?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")
        .map_err(sqlite("enable foreign keys"))?;

    let current: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(sqlite("read schema version"))?;
    let schema_reset = current != SCHEMA_VERSION;
    if schema_reset {
        if current != 0 {
            tracing::info!(
                old = current,
                new = SCHEMA_VERSION,
                "schema version mismatch — recreating database"
            );
        }
        drop_all_tables(&conn)?;
        conn.execute_batch(SCHEMA).map_err(sqlite("apply schema"))?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(sqlite("set schema version"))?;
    }

    Ok(DbOpenResult {
        db: Db::from(rekindle_asql::Connection::from(conn)),
        schema_reset,
    })
}

/// Drop every user table so the schema can be re-applied.
///
/// Virtual tables (FTS5) go first: they own shadow tables (`<name>_data`,
/// `<name>_idx`, …) that SQLite refuses to drop directly. They are the rows
/// whose `sql` starts `CREATE VIRTUAL`; shadow tables have a NULL `sql`.
fn drop_all_tables(conn: &Connection) -> Result<(), DbError> {
    conn.execute_batch("PRAGMA foreign_keys=OFF;")
        .map_err(sqlite("disable foreign keys"))?;
    for kind in ["CREATE VIRTUAL%", "CREATE TABLE%"] {
        for table in tables_whose_sql_is_like(conn, kind)? {
            conn.execute_batch(&format!("DROP TABLE IF EXISTS \"{table}\";"))
                .map_err(sqlite("drop table"))?;
        }
    }
    conn.execute_batch("PRAGMA foreign_keys=ON;")
        .map_err(sqlite("re-enable foreign keys"))
}

fn tables_whose_sql_is_like(conn: &Connection, pattern: &str) -> Result<Vec<String>, DbError> {
    let mut stmt = conn
        .prepare(
            "SELECT name FROM sqlite_master \
             WHERE type = 'table' AND sql LIKE ?1 AND name NOT LIKE 'sqlite_%'",
        )
        .map_err(sqlite("list tables"))?;
    let names = stmt
        .query_map([pattern], |row| row.get(0))
        .map_err(sqlite("list tables"))?
        .collect::<Result<Vec<String>, _>>()
        .map_err(sqlite("list tables"))?;
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_new_database_gets_the_schema_and_its_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rekindle.db");
        let first = open(&path).unwrap();
        assert!(first.schema_reset, "a new file has no schema yet");
        let version: i64 = first
            .db
            .call(|c| c.pragma_query_value(None, "user_version", |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        drop(first);
        assert!(!open(&path).unwrap().schema_reset, "reopening keeps it");
    }

    #[test]
    fn a_stale_version_recreates_every_table() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rekindle.db");
        drop(open(&path).unwrap());
        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "user_version", SCHEMA_VERSION - 1)
            .unwrap();
        drop(conn);
        assert!(open(&path).unwrap().schema_reset);
    }
}
