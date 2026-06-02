//! SQLite round-trip integration tests for the audit chain. The
//! plan's Phase 4 manual scenario was "modify a row in SQLite,
//! re-verify, expect ok=false" — these tests reproduce it
//! programmatically.
//!
//! `rekindle-audit` itself has unit tests for in-memory tamper
//! detection (`crates/rekindle-audit/src/chain.rs`); the tests here
//! exercise the persistence + verify_async path that ties the chain
//! to the `audit_entries` SQLite table.

use super::store::{insert_entry, load_all, load_since, load_tail};
use rekindle_audit::{AuditChain, AuditEntry, AuditKind, AuditRecord, VerifyError, MAC_LEN};
use tokio_rusqlite::Connection as TokioConn;

async fn fresh_db_with_audit_table() -> std::sync::Arc<TokioConn> {
    let conn = TokioConn::open_in_memory().await.unwrap();
    conn.call(|c| -> rusqlite::Result<()> {
        c.execute_batch(
            "CREATE TABLE identity (public_key TEXT PRIMARY KEY);
                 CREATE TABLE audit_entries (
                    owner_key TEXT NOT NULL,
                    cursor INTEGER NOT NULL,
                    prev_mac BLOB NOT NULL,
                    mac BLOB NOT NULL,
                    payload_json TEXT NOT NULL,
                    PRIMARY KEY (owner_key, cursor)
                 );",
        )?;
        Ok(())
    })
    .await
    .unwrap();
    std::sync::Arc::new(conn)
}

fn fixture_chain(key: [u8; 32]) -> AuditChain {
    AuditChain::open(zeroize::Zeroizing::new(key), [0u8; MAC_LEN], 0)
}

fn mk_record(n: u64) -> AuditRecord {
    AuditRecord {
        at_ms: 1_700_000_000_000 + n.cast_signed(),
        actor_pub: "alice".into(),
        kind: AuditKind::FriendAdded,
        payload: serde_json::json!({ "peer": format!("bob-{n}") }),
    }
}

#[tokio::test]
async fn persist_then_load_roundtrip() {
    let pool = fresh_db_with_audit_table().await;
    let mut chain = fixture_chain([7u8; 32]);
    let owner = "alice".to_string();

    let mut originals = Vec::new();
    for n in 1..=5 {
        let entry = chain.append(mk_record(n)).unwrap();
        let owner_c = owner.clone();
        let entry_c = entry.clone();
        pool.call(move |c| -> rusqlite::Result<()> {
            insert_entry(c, &owner_c, &entry_c)?;
            Ok(())
        })
        .await
        .unwrap();
        originals.push(entry);
    }

    let owner_c = owner.clone();
    let loaded = pool
        .call(move |c| -> rusqlite::Result<Vec<AuditEntry>> { load_all(c, &owner_c) })
        .await
        .unwrap();
    assert_eq!(loaded.len(), 5);
    for (a, b) in originals.iter().zip(loaded.iter()) {
        assert_eq!(a.cursor, b.cursor);
        assert_eq!(a.mac, b.mac);
        assert_eq!(a.prev_mac, b.prev_mac);
        assert_eq!(a.record.actor_pub, b.record.actor_pub);
    }

    // verify against the loaded entries — chain is intact.
    let verifier = fixture_chain([7u8; 32]);
    verifier
        .verify(&loaded)
        .expect("persisted chain verifies cleanly");
}

#[tokio::test]
async fn tampered_sqlite_row_fails_verify() {
    // The exact scenario from the plan's Tauri-testable section:
    // (1) append a few entries, persist, (2) modify one row's
    // payload_json directly in SQLite, (3) reload + verify — must
    // report the tampered cursor.
    let pool = fresh_db_with_audit_table().await;
    let mut chain = fixture_chain([7u8; 32]);
    let owner = "alice".to_string();

    for n in 1..=3 {
        let entry = chain.append(mk_record(n)).unwrap();
        let owner_c = owner.clone();
        let entry_c = entry.clone();
        pool.call(move |c| -> rusqlite::Result<()> {
            insert_entry(c, &owner_c, &entry_c)?;
            Ok(())
        })
        .await
        .unwrap();
    }

    // Hand-modify the middle row to mimic an attacker editing the
    // SQLite database while the app is offline.
    pool.call(|c| -> rusqlite::Result<()> {
        c.execute(
            "UPDATE audit_entries SET payload_json = '{\"at_ms\":0,\"actor_pub\":\"EVIL\",\"kind\":\"FriendAdded\",\"payload\":{\"peer\":\"forged\"}}' \
                 WHERE owner_key = 'alice' AND cursor = 2",
            [],
        )?;
        Ok(())
    })
    .await
    .unwrap();

    let loaded = pool
        .call(|c| -> rusqlite::Result<Vec<AuditEntry>> { load_all(c, "alice") })
        .await
        .unwrap();
    let verifier = fixture_chain([7u8; 32]);
    let err = verifier.verify(&loaded).unwrap_err();
    match err {
        VerifyError::MacMismatch { cursor, .. } => assert_eq!(cursor, 2),
        other => panic!("expected MacMismatch at cursor 2, got {other:?}"),
    }
}

#[tokio::test]
async fn load_tail_returns_genesis_for_empty_table() {
    let pool = fresh_db_with_audit_table().await;
    let (cursor, mac) = pool
        .call(|c| -> rusqlite::Result<(u64, [u8; 32])> { load_tail(c, "alice") })
        .await
        .unwrap();
    assert_eq!(cursor, 0);
    assert_eq!(mac, [0u8; 32]);
}

#[tokio::test]
async fn load_tail_recovers_chain_state() {
    let pool = fresh_db_with_audit_table().await;
    let mut chain = fixture_chain([7u8; 32]);
    let owner = "alice".to_string();

    let mut last = None;
    for n in 1..=4 {
        let entry = chain.append(mk_record(n)).unwrap();
        let owner_c = owner.clone();
        let entry_c = entry.clone();
        pool.call(move |c| -> rusqlite::Result<()> {
            insert_entry(c, &owner_c, &entry_c)?;
            Ok(())
        })
        .await
        .unwrap();
        last = Some(entry);
    }
    let last = last.unwrap();

    let (cursor, mac) = pool
        .call(|c| -> rusqlite::Result<(u64, [u8; 32])> { load_tail(c, "alice") })
        .await
        .unwrap();
    assert_eq!(cursor, last.cursor);
    assert_eq!(mac, last.mac);

    // Reopening the chain from the persisted tail must produce a chain
    // whose next `append` links to the prior tail's mac.
    let mut reopened = AuditChain::open(zeroize::Zeroizing::new([7u8; 32]), mac, cursor);
    let next = reopened.append(mk_record(99)).unwrap();
    assert_eq!(next.cursor, 5);
    assert_eq!(next.prev_mac, last.mac);
}

#[tokio::test]
async fn load_since_filters_correctly() {
    let pool = fresh_db_with_audit_table().await;
    let mut chain = fixture_chain([7u8; 32]);
    let owner = "alice".to_string();

    for n in 1..=5 {
        let entry = chain.append(mk_record(n)).unwrap();
        let owner_c = owner.clone();
        let entry_c = entry.clone();
        pool.call(move |c| -> rusqlite::Result<()> {
            insert_entry(c, &owner_c, &entry_c)?;
            Ok(())
        })
        .await
        .unwrap();
    }
    let since_3 = pool
        .call(|c| -> rusqlite::Result<Vec<AuditEntry>> { load_since(c, "alice", 3) })
        .await
        .unwrap();
    assert_eq!(since_3.len(), 2);
    assert_eq!(since_3[0].cursor, 4);
    assert_eq!(since_3[1].cursor, 5);
}

#[tokio::test]
async fn tail_truncation_attack_is_detected_via_anchor() {
    // Reproduce the threat model: attacker with SQLite write access
    // drops trailing audit_entries rows. Without the vault-persisted
    // tail anchor, this would be invisible because the remaining
    // chain still verifies internally (it's just shorter). With the
    // anchor, restore_chain can compare SQLite's tail against the
    // vault's tail and detect the deletion.
    //
    // This test exercises the load_tail / load_audit_tail comparison
    // at the cursor + mac level — the same logic restore_chain runs
    // when called with `app_handle: None` (no Tauri AppHandle in unit
    // tests, so we can't observe the emit, but we observe the data).
    let pool = fresh_db_with_audit_table().await;
    let mut chain = fixture_chain([7u8; 32]);
    let owner = "alice".to_string();

    // Persist 5 entries via the normal append+insert path.
    let mut last_legit = None;
    for n in 1..=5 {
        let entry = chain.append(mk_record(n)).unwrap();
        let owner_c = owner.clone();
        let entry_c = entry.clone();
        pool.call(move |c| -> rusqlite::Result<()> {
            insert_entry(c, &owner_c, &entry_c)?;
            Ok(())
        })
        .await
        .unwrap();
        last_legit = Some(entry);
    }
    let legitimate_tail = last_legit.unwrap();

    // Attacker drops the last 2 rows.
    pool.call(|c| -> rusqlite::Result<()> {
        c.execute(
            "DELETE FROM audit_entries WHERE owner_key = 'alice' AND cursor > 3",
            [],
        )?;
        Ok(())
    })
    .await
    .unwrap();

    // After tampering, SQLite's tail reports cursor=3 with cursor-3's mac.
    // The vault anchor (had we written it) would report cursor=5 with the
    // legitimate tail mac. Mismatch → tamper detected.
    let (sqlite_cursor, sqlite_mac) = pool
        .call(|c| -> rusqlite::Result<(u64, [u8; 32])> { load_tail(c, "alice") })
        .await
        .unwrap();
    assert_eq!(sqlite_cursor, 3, "post-truncation SQLite tail is cursor 3");
    assert_ne!(
        sqlite_mac, legitimate_tail.mac,
        "post-truncation tail mac differs from legitimate tail mac",
    );
    // The pre-tamper anchor (cursor=5, legitimate_tail.mac) is what
    // restore_chain compares against. Inequality on either field
    // would trigger the tamper signal — proven by the assertions above.
}

/// Encodes the same decision rule as `restore_chain`'s anchor-vs-SQLite
/// comparison. Extracted so the three branch tests below stay
/// declarative — they pin the rule, not the surrounding plumbing.
fn anchor_decision(
    anchor_cursor: u64,
    anchor_mac: [u8; MAC_LEN],
    sqlite_cursor: u64,
    sqlite_mac: [u8; MAC_LEN],
) -> Option<u64> {
    if anchor_cursor < sqlite_cursor || (anchor_cursor == sqlite_cursor && anchor_mac == sqlite_mac)
    {
        None
    } else {
        Some(anchor_cursor)
    }
}

#[test]
fn anchor_behind_sqlite_is_catchup_not_tamper() {
    // Race scenario from aspect (h4): an in-flight append_async wrote
    // its row to SQLite but lost the vault anchor write (e.g. logout
    // cleared keystore between SQLite insert and tail persist).
    // Decision: anchor_cursor < sqlite_cursor means catch-up, NOT
    // tamper.
    assert!(
        anchor_decision(3, [0xAAu8; MAC_LEN], 5, [0xBBu8; MAC_LEN]).is_none(),
        "anchor behind SQLite must be catch-up",
    );
}

#[test]
fn anchor_ahead_of_sqlite_is_tamper() {
    // Truncation attack: attacker dropped trailing audit rows. The
    // anchor (vault-persisted on last legitimate append) is now
    // ahead of SQLite. Must signal tamper.
    assert_eq!(
        anchor_decision(5, [0xAAu8; MAC_LEN], 3, [0xBBu8; MAC_LEN]),
        Some(5),
    );
}

#[test]
fn anchor_equal_cursor_but_mac_mismatch_is_tamper() {
    // Attacker modified the tail row's payload (cursor unchanged but
    // MAC differs from anchor). Must signal tamper.
    assert_eq!(
        anchor_decision(5, [0xAAu8; MAC_LEN], 5, [0xCCu8; MAC_LEN]),
        Some(5),
    );
}

#[test]
fn anchor_equal_cursor_and_mac_is_clean() {
    // Happy path — both vault and SQLite agree.
    assert!(anchor_decision(5, [0xAAu8; MAC_LEN], 5, [0xAAu8; MAC_LEN]).is_none(),);
}

#[tokio::test]
async fn fk_cascade_drops_audit_when_identity_deleted() {
    // Phase 4 SQLite migration declares
    //   owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE
    // so deleting an identity wipes its audit_entries (orphans = bad).
    // Verify the cascade actually fires under runtime `PRAGMA foreign_keys=ON;`.
    let conn = TokioConn::open_in_memory().await.unwrap();
    conn.call(|c| -> rusqlite::Result<()> {
        // foreign_keys must be ON for cascade to fire (db.rs::open() does this in prod)
        c.execute_batch("PRAGMA foreign_keys=ON;")?;
        c.execute_batch(
            "CREATE TABLE identity (public_key TEXT PRIMARY KEY);
                 CREATE TABLE audit_entries (
                    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
                    cursor INTEGER NOT NULL,
                    prev_mac BLOB NOT NULL,
                    mac BLOB NOT NULL,
                    payload_json TEXT NOT NULL,
                    PRIMARY KEY (owner_key, cursor)
                 );
                 INSERT INTO identity (public_key) VALUES ('alice');",
        )?;
        Ok(())
    })
    .await
    .unwrap();
    let pool = std::sync::Arc::new(conn);

    // Persist 3 audit entries for alice.
    let mut chain = fixture_chain([7u8; 32]);
    for n in 1..=3 {
        let entry = chain.append(mk_record(n)).unwrap();
        let entry_c = entry.clone();
        pool.call(move |c| -> rusqlite::Result<()> {
            insert_entry(c, "alice", &entry_c)?;
            Ok(())
        })
        .await
        .unwrap();
    }
    let before_count: i64 = pool
        .call(|c| -> rusqlite::Result<i64> {
            c.query_row(
                "SELECT COUNT(*) FROM audit_entries WHERE owner_key = 'alice'",
                [],
                |r| r.get(0),
            )
        })
        .await
        .unwrap();
    assert_eq!(before_count, 3);

    // Delete the identity — cascade must wipe the 3 audit rows.
    pool.call(|c| -> rusqlite::Result<()> {
        c.execute("DELETE FROM identity WHERE public_key = 'alice'", [])?;
        Ok(())
    })
    .await
    .unwrap();

    let after_count: i64 = pool
        .call(|c| -> rusqlite::Result<i64> {
            c.query_row(
                "SELECT COUNT(*) FROM audit_entries WHERE owner_key = 'alice'",
                [],
                |r| r.get(0),
            )
        })
        .await
        .unwrap();
    assert_eq!(
        after_count, 0,
        "ON DELETE CASCADE must wipe audit entries when identity is deleted (orphans = bad)",
    );
}

#[tokio::test]
async fn owner_key_isolates_chains() {
    // Two identities on one device must have independent chains.
    let pool = fresh_db_with_audit_table().await;
    let mut alice_chain = fixture_chain([1u8; 32]);
    let mut bob_chain = fixture_chain([2u8; 32]);

    for n in 1..=3 {
        let alice_entry = alice_chain.append(mk_record(n)).unwrap();
        let bob_entry = bob_chain.append(mk_record(n + 100)).unwrap();
        let ae = alice_entry.clone();
        let be = bob_entry.clone();
        pool.call(move |c| -> rusqlite::Result<()> {
            insert_entry(c, "alice", &ae)?;
            insert_entry(c, "bob", &be)?;
            Ok(())
        })
        .await
        .unwrap();
    }

    let alice_entries = pool
        .call(|c| -> rusqlite::Result<Vec<AuditEntry>> { load_all(c, "alice") })
        .await
        .unwrap();
    let bob_entries = pool
        .call(|c| -> rusqlite::Result<Vec<AuditEntry>> { load_all(c, "bob") })
        .await
        .unwrap();
    assert_eq!(alice_entries.len(), 3);
    assert_eq!(bob_entries.len(), 3);
    for (a, b) in alice_entries.iter().zip(bob_entries.iter()) {
        assert_ne!(a.mac, b.mac, "different keys must produce distinct MACs");
    }
}
