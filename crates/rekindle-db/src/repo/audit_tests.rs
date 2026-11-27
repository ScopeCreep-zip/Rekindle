//! The chain against the real schema: what is stored verifies, a row
//! edited on disk does not, and dropped rows show against the anchor.

use super::*;
use crate::repo::fixture::{self, OWNER};
use rekindle_audit::{AuditChain, AuditKind, TailCheck, VerifyError, MAC_LEN};

fn chain(key: u8) -> AuditChain {
    AuditChain::open(zeroize::Zeroizing::new([key; 32]), [0; MAC_LEN], 0)
}

fn record(n: u64) -> AuditRecord {
    AuditRecord {
        at_ms: 1_700_000_000_000 + n.cast_signed(),
        actor_pub: OWNER.into(),
        kind: AuditKind::FriendAdded,
        payload: serde_json::json!({ "peer": format!("bob-{n}") }),
    }
}

/// Append `n` records for `owner` and store them; the entries in order.
fn store(conn: &Connection, owner: &str, chain: &mut AuditChain, n: u64) -> Vec<AuditEntry> {
    (1..=n)
        .map(|i| {
            let entry = chain.append(record(i)).unwrap();
            insert_entry(conn, owner, &entry).unwrap();
            entry
        })
        .collect()
}

#[test]
fn stored_entries_load_back_and_verify() {
    let conn = fixture::conn();
    let originals = store(&conn, OWNER, &mut chain(7), 5);
    let loaded = load_all(&conn, OWNER).unwrap();
    assert_eq!(loaded.len(), 5);
    for (a, b) in originals.iter().zip(&loaded) {
        assert_eq!((a.cursor, a.mac, a.prev_mac), (b.cursor, b.mac, b.prev_mac));
        assert_eq!(a.record.actor_pub, b.record.actor_pub);
    }
    chain(7).verify(&loaded).unwrap();
}

#[test]
fn a_row_edited_on_disk_fails_verify_at_its_cursor() {
    let conn = fixture::conn();
    store(&conn, OWNER, &mut chain(7), 3);
    conn.execute(
        "UPDATE audit_entries SET payload_json = \
             '{\"at_ms\":0,\"actor_pub\":\"EVIL\",\"kind\":\"FriendAdded\",\"payload\":{}}' \
             WHERE owner_key = ?1 AND cursor = 2",
        [OWNER],
    )
    .unwrap();
    match chain(7).verify(&load_all(&conn, OWNER).unwrap()) {
        Err(VerifyError::MacMismatch { cursor, .. }) => assert_eq!(cursor, 2),
        other => panic!("expected MacMismatch at cursor 2, got {other:?}"),
    }
}

#[test]
fn an_empty_chain_has_the_genesis_tail() {
    let conn = fixture::conn();
    assert_eq!(load_tail(&conn, OWNER).unwrap(), (0, [0; 32]));
}

#[test]
fn the_stored_tail_resumes_the_chain() {
    let conn = fixture::conn();
    let last = store(&conn, OWNER, &mut chain(7), 4).pop().unwrap();
    let (cursor, mac) = load_tail(&conn, OWNER).unwrap();
    assert_eq!((cursor, mac), (last.cursor, last.mac));
    let mut reopened = AuditChain::open(zeroize::Zeroizing::new([7; 32]), mac, cursor);
    let next = reopened.append(record(99)).unwrap();
    assert_eq!((next.cursor, next.prev_mac), (5, last.mac));
}

#[test]
fn load_since_returns_only_later_cursors() {
    let conn = fixture::conn();
    store(&conn, OWNER, &mut chain(7), 5);
    let cursors: Vec<u64> = load_since(&conn, OWNER, 3)
        .unwrap()
        .iter()
        .map(|e| e.cursor)
        .collect();
    assert_eq!(cursors, [4, 5]);
}

/// Dropped trailing rows leave a chain that still verifies; the vault
/// anchor written by the last real append is what catches it.
#[test]
fn dropped_tail_rows_show_against_the_anchor() {
    let conn = fixture::conn();
    let anchor_entry = store(&conn, OWNER, &mut chain(7), 5).pop().unwrap();
    conn.execute(
        "DELETE FROM audit_entries WHERE owner_key = ?1 AND cursor > 3",
        [OWNER],
    )
    .unwrap();
    chain(7).verify(&load_all(&conn, OWNER).unwrap()).unwrap();
    let anchor = (anchor_entry.cursor, anchor_entry.mac);
    let stored = load_tail(&conn, OWNER).unwrap();
    assert_eq!(
        TailCheck::of(Some(anchor), stored),
        TailCheck::Tampered { anchor }
    );
}

#[test]
fn deleting_the_identity_deletes_its_chain() {
    let conn = fixture::conn();
    store(&conn, OWNER, &mut chain(7), 3);
    conn.execute("DELETE FROM identity WHERE public_key = ?1", [OWNER])
        .unwrap();
    assert!(load_all(&conn, OWNER).unwrap().is_empty());
}

#[test]
fn each_owner_has_its_own_chain() {
    let conn = fixture::conn();
    conn.execute(
        "INSERT INTO identity (public_key, created_at, friend_request_token) VALUES ('other', 0, 'aaaaaaaaaaaaaaaaaaaaaaaaaa')",
        [],
    )
    .unwrap();
    store(&conn, OWNER, &mut chain(1), 3);
    store(&conn, "other", &mut chain(2), 3);
    let mine = load_all(&conn, OWNER).unwrap();
    let theirs = load_all(&conn, "other").unwrap();
    assert_eq!((mine.len(), theirs.len()), (3, 3));
    for (a, b) in mine.iter().zip(&theirs) {
        assert_ne!(a.mac, b.mac);
    }
}
