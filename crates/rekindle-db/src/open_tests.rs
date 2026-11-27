use rusqlite::OptionalExtension;

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

/// The durability settings (R3: the ratchet commits before the send):
/// `synchronous=FULL` (2), and both full-fsync flags, which SQLite
/// honours on macOS only but reports everywhere; and the WAL is cut to
/// nothing at each reset (`journal_size_limit=0`).
#[tokio::test]
async fn a_new_database_syncs_fully() {
    let dir = tempfile::tempdir().unwrap();
    let db = open(&dir.path().join("pk.db"), &[7; 32]).unwrap();
    let pragmas: (i64, i64, i64, i64) = db
        .call(|c| {
            let get = |name: &str| c.pragma_query_value(None, name, |r| r.get::<_, i64>(0));
            Ok((
                get("synchronous")?,
                get("fullfsync")?,
                get("checkpoint_fullfsync")?,
                get("journal_size_limit")?,
            ))
        })
        .await
        .unwrap();
    assert_eq!(pragmas, (2, 1, 1, 0));
}

/// The linked SQLite is SQLCipher: `bundled-sqlcipher-vendored-openssl`
/// must keep winning feature unification over the `bundled` that
/// veilid-core's keyvaluedb-sqlite → async-sqlite chain asks for (plan
/// VU1, synthesis #17). Stock SQLite answers no row to this pragma.
#[tokio::test]
async fn the_linked_sqlite_is_sqlcipher() {
    let dir = tempfile::tempdir().unwrap();
    let db = open(&dir.path().join("pk.db"), &[7; 32]).unwrap();
    let version: Option<String> = db
        .call(|c| {
            c.query_row("PRAGMA cipher_version", [], |r| r.get(0))
                .optional()
        })
        .await
        .unwrap();
    assert!(
        version.is_some_and(|v| !v.is_empty()),
        "PRAGMA cipher_version is empty: the build links stock SQLite"
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
