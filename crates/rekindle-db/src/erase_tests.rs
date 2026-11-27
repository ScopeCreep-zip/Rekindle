use super::*;

fn wal_len(dir: &tempfile::TempDir) -> u64 {
    std::fs::metadata(dir.path().join("i.db-wal")).map_or(0, |m| m.len())
}

/// I14's window: an erase's frames leave the `-wal` file by the next
/// call on the database (the truncation is queued behind the erasing
/// call, never inside it), and a write that erased nothing leaves the
/// WAL as it is.
#[tokio::test]
async fn an_erase_leaves_the_wal_by_the_next_call() {
    let dir = tempfile::tempdir().unwrap();
    let db = crate::open(&dir.path().join("i.db"), &[7; 32]).unwrap();
    db.call(|conn| {
        conn.execute_batch(
            "CREATE TABLE secrets (k BLOB); INSERT INTO secrets VALUES (x'0102030405');",
        )
    })
    .await
    .unwrap();
    db.call(|_| Ok(())).await.unwrap();
    assert!(
        wal_len(&dir) > 0,
        "a write that erased nothing keeps its frames"
    );

    let deleted = db
        .call(|conn| delete(conn, "DELETE FROM secrets", []))
        .await
        .unwrap();
    assert_eq!(deleted, 1);
    db.call(|_| Ok(())).await.unwrap();
    assert_eq!(wal_len(&dir), 0, "the erase's frames are gone");
}

/// An erase inside a caller's transaction counts at that caller's
/// commit, whoever opened it (RV5 M3).
#[tokio::test]
async fn a_nested_erase_truncates_after_the_outermost_commit() {
    let dir = tempfile::tempdir().unwrap();
    let db = crate::open(&dir.path().join("i.db"), &[7; 32]).unwrap();
    db.call(|conn| {
        conn.execute_batch("CREATE TABLE secrets (k BLOB); INSERT INTO secrets VALUES (x'01');")
    })
    .await
    .unwrap();
    db.call(|conn| {
        let tx = conn.transaction()?;
        delete(&tx, "DELETE FROM secrets", [])?;
        assert!(!take_committed(&tx), "not taken inside the transaction");
        tx.commit()
    })
    .await
    .unwrap();
    db.call(|_| Ok(())).await.unwrap();
    assert_eq!(wal_len(&dir), 0);
}
