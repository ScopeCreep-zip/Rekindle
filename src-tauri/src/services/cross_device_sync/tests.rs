//! Smoke tests for the cross-device sync orchestration. The crypto
//! itself has unit tests in `rekindle-secrets::sync_key`; the merge
//! rules have tests in `merge.rs`. This file proves the service-level
//! plumbing serializes and round-trips correctly through the FTS5-era
//! schema (so the DB columns are present and writable).

use rusqlite::Connection;

const MIGRATION: &str = include_str!("../../../migrations/001_init.sql");

fn open_db() -> Connection {
    let conn = Connection::open_in_memory().expect("open in-memory db");
    conn.execute_batch(MIGRATION).expect("apply migration");
    conn.execute(
        "INSERT INTO identity (id, public_key, created_at) VALUES (1, 'owner_pk', 0)",
        [],
    )
    .expect("seed identity");
    conn
}

#[test]
fn identity_table_has_personal_sync_columns() {
    let conn = open_db();
    conn.execute(
        "UPDATE identity SET personal_sync_record_key = 'rk', \
             personal_sync_owner_keypair = 'kp', device_id = 'd1' \
          WHERE public_key = 'owner_pk'",
        [],
    )
    .expect("update");
    let value: String = conn
        .query_row(
            "SELECT personal_sync_record_key FROM identity WHERE public_key = 'owner_pk'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(value, "rk");
}

#[test]
fn paired_devices_table_round_trips() {
    let conn = open_db();
    conn.execute(
        "INSERT INTO paired_devices (owner_key, device_id, device_public_key, display_name, paired_at) \
         VALUES ('owner_pk', 'dev1', 'pk1', 'Laptop', 100)",
        [],
    )
    .expect("insert");
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM paired_devices WHERE owner_key = 'owner_pk'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn channel_read_state_row_writes_and_reads() {
    let conn = open_db();
    conn.execute(
        "INSERT INTO channel_read_state (owner_key, community_id, channel_id, last_read_lamport, updated_at) \
         VALUES ('owner_pk', 'c1', 'ch1', 42, 100)",
        [],
    )
    .expect("insert");
    let lamport: i64 = conn
        .query_row(
            "SELECT last_read_lamport FROM channel_read_state \
              WHERE owner_key = 'owner_pk' AND community_id = 'c1' AND channel_id = 'ch1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(lamport, 42);
}

#[test]
fn pending_pairings_table_with_blob_salt() {
    let conn = open_db();
    let salt = vec![0xAAu8; 16];
    conn.execute(
        "INSERT INTO pending_pairings (owner_key, pairing_code, pairing_salt, created_at, expires_at) \
         VALUES ('owner_pk', 'CODE-1234', ?1, 100, 200)",
        rusqlite::params![salt],
    )
    .expect("insert");
    let recovered: Vec<u8> = conn
        .query_row(
            "SELECT pairing_salt FROM pending_pairings WHERE owner_key = 'owner_pk'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(recovered, salt);
}
