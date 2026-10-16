//! Opening an identity's database: the key, pragmas and the schema-version
//! check (plan D1).

use std::path::Path;

use rusqlite::Connection;

use crate::Db;

/// The schema. Edited in place before release; bump [`SCHEMA_VERSION`]
/// with every change (CLAUDE.md "No migration files").
pub const SCHEMA: &str = include_str!("../schema/001_init.sql");

/// Bump this every time `schema/001_init.sql` changes, or when a DHT record
/// layout is renumbered. It is also the data root's layout version
/// (`crate::reset`): a root stamped with another value is reset at startup,
/// Veilid storage included, so stale records written under the old layout
/// cannot be read back with the new indices. The app is not live yet.
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
/// 84: `app_settings.link_previews_enabled` defaults to 0, so a row
/// another setting creates no longer turns link previews (an IP leak to
/// the linked site) on (plan C7.28).
///
/// 85: the identity store (plan D1). One encrypted database per identity;
/// the identity's record owner keypairs moved to its vault (their
/// `identity` columns and `friends.local_conversation_keypair` dropped);
/// `identity.friend_inbox_{key,keypair}` and `friends.dm_log_key` hold
/// the daemon's friend-inbox protocol until E2.1.
///
/// 86: `messages.send_state` (D22's delivery vocabulary) and the held
/// write's slot (`held_record_key`, `held_subkey`), plan D1 / C7.14.
///
/// 87: the preferences move from the plugin store into `app_settings`
/// columns and `app_input_channels` (plan D1).
pub const SCHEMA_VERSION: i64 = 87;

/// Opening the database failed.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// SQLite refused an operation; `context` says which.
    #[error("{context}: {source}")]
    Sqlite {
        context: &'static str,
        source: rusqlite::Error,
    },
    /// The key does not decrypt the identity database.
    #[error("the database key does not open this database")]
    WrongKey,
    /// The identity database was written for another schema version.
    #[error("the database is at schema version {found}, not {expected}", expected = SCHEMA_VERSION)]
    VersionMismatch { found: i64 },
    /// A file operation on the database failed.
    #[error("database file: {0}")]
    Io(std::io::Error),
}

fn sqlite(context: &'static str) -> impl FnOnce(rusqlite::Error) -> DbError {
    move |source| DbError::Sqlite { context, source }
}

/// Open an identity's encrypted database (plan D1): keyed with the
/// vault-derived `key` (SQLCipher), verified, WAL, foreign keys and
/// `secure_delete` on. A new database gets the schema; a database at
/// another schema version is an error, never a silent reset — the layout
/// check (`crate::reset::check_layout`) already cleared a root laid out for
/// another version.
///
/// # Errors
/// [`DbError::WrongKey`] when `key` does not decrypt it,
/// [`DbError::VersionMismatch`], or the SQLite step that failed.
pub fn open(path: &Path, key: &[u8; 32]) -> Result<Db, DbError> {
    let conn = Connection::open(path).map_err(sqlite("open database"))?;
    conn.pragma_update(None, "key", format!("x'{}'", hex_lower(key)))
        .map_err(sqlite("key database"))?;
    let tables: i64 = conn
        .query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0))
        .map_err(|e| match e.sqlite_error_code() {
            Some(rusqlite::ErrorCode::NotADatabase | rusqlite::ErrorCode::OutOfMemory) => {
                DbError::WrongKey
            }
            _ => DbError::Sqlite {
                context: "verify key",
                source: e,
            },
        })?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA secure_delete=ON;")
        .map_err(sqlite("set pragmas"))?;
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(sqlite("read schema version"))?;
    if version == 0 && tables == 0 {
        conn.execute_batch(SCHEMA).map_err(sqlite("apply schema"))?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(sqlite("set schema version"))?;
    } else if version != SCHEMA_VERSION {
        return Err(DbError::VersionMismatch { found: version });
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(DbError::Io)?;
    }
    Ok(Db::from(rekindle_asql::Connection::from(conn)))
}

fn hex_lower(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_identity_database_opens_only_with_its_key_and_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pk.db");
        let db = open(&path, &[7; 32]).unwrap();
        let version: i64 = db
            .call(|c| c.pragma_query_value(None, "user_version", |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        drop(db);
        assert!(matches!(open(&path, &[8; 32]), Err(DbError::WrongKey)));
        assert!(open(&path, &[7; 32]).is_ok());
        let plain = std::fs::read(&path).unwrap();
        assert!(
            !plain.starts_with(b"SQLite format 3"),
            "the file is encrypted"
        );
    }

    #[test]
    fn an_identity_database_at_another_version_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pk.db");
        drop(open(&path, &[7; 32]).unwrap());
        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "key", format!("x'{}'", hex_lower(&[7; 32])))
            .unwrap();
        conn.pragma_update(None, "user_version", SCHEMA_VERSION - 1)
            .unwrap();
        drop(conn);
        assert!(matches!(
            open(&path, &[7; 32]),
            Err(DbError::VersionMismatch { found }) if found == SCHEMA_VERSION - 1
        ));
    }
}
