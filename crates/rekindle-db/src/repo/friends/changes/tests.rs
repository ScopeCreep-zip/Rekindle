use super::*;
use crate::repo::fixture::{conn, OWNER};

const BOB: &str = "bb";

fn sessions(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM ratchet_sessions", [], |r| r.get(0))
        .unwrap()
}

/// Plan E2.5h: remove, block and re-add never delete sessions; their
/// keys are erased only by eviction from the archive or an identity
/// reset.
#[test]
fn ending_a_friendship_keeps_its_sessions() {
    let conn = conn();
    conn.execute(
        "INSERT INTO ratchet_sessions (owner_key, peer_identity, peer_device, record, updated_at) \
         VALUES (?1, ?2, 'd', x'00', 1)",
        params![OWNER.to_ascii_lowercase(), BOB],
    )
    .unwrap();
    super::super::insert_pending_out(&conn, OWNER, BOB, "Bob", 1).unwrap();
    cancel_request(&conn, OWNER, BOB).unwrap();
    insert_accepted(&conn, OWNER, BOB, "Bob", 2).unwrap();
    unfriended(&conn, OWNER, BOB).unwrap();
    insert_accepted(&conn, OWNER, BOB, "Bob", 3).unwrap();
    set_state(&conn, OWNER, BOB, FriendshipState::Removing).unwrap();
    forget_removed(&conn, OWNER).unwrap();
    insert_accepted(&conn, OWNER, BOB, "Bob", 4).unwrap();
    block(&conn, OWNER, BOB, "Bob", 5).unwrap();
    assert_eq!(sessions(&conn), 1);
}

/// I14 (RV5) with plan E2.5h: ending, cancelling or re-adding a
/// friendship erases no session, so none of these changes runs a
/// checkpoint, and the stored session is still there afterwards.
#[tokio::test]
async fn ending_a_friendship_erases_nothing_and_truncates_nothing() {
    type Change = fn(&Connection) -> rusqlite::Result<()>;
    let cases: [(&str, Change, Change); 7] = [
        ("cancel_request", pending, |c| {
            cancel_request(c, OWNER, BOB).map(drop)
        }),
        ("block", accepted, |c| {
            block(c, OWNER, BOB, "Bob", 2).map(drop)
        }),
        ("unfriended", accepted, |c| {
            unfriended(c, OWNER, BOB).map(drop)
        }),
        ("forget_removed", removing, |c| {
            forget_removed(c, OWNER).map(drop)
        }),
        ("expiry", pending, |c| {
            super::super::delete_pending_out_before(c, OWNER, 5).map(drop)
        }),
        ("insert_accepted", removing, |c| {
            insert_accepted(c, OWNER, BOB, "Bob", 3).map(drop)
        }),
        ("insert_pending_out", removing, |c| {
            super::super::insert_pending_out(c, OWNER, BOB, "Bob", 3).map(drop)
        }),
    ];
    for (name, setup, change) in cases {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::open(&dir.path().join("o.db"), &[7; 32]).unwrap();
        let wal = dir.path().join("o.db-wal");
        let wal_len = || std::fs::metadata(&wal).unwrap().len();
        db.call(move |c| {
            crate::repo::identity::insert(c, OWNER, "me", 0, "aaaaaaaaaaaaaaaaaaaaaaaaaa")?;
            setup(c)?;
            store_session(c)
        })
        .await
        .unwrap();
        let written = wal_len();
        assert!(written > 0, "{name}: the setup wrote WAL frames");
        db.call(move |c| change(c)).await.unwrap();
        let kept = db
            .call(|c| {
                c.query_row("SELECT COUNT(*) FROM ratchet_sessions", [], |r| {
                    r.get::<_, i64>(0)
                })
            })
            .await
            .unwrap();
        assert_eq!(kept, 1, "{name}: the session stays");
        assert!(
            wal_len() >= written,
            "{name}: erased nothing, yet truncated"
        );
    }
}

fn pending(c: &Connection) -> rusqlite::Result<()> {
    super::super::insert_pending_out(c, OWNER, BOB, "Bob", 1).map(drop)
}

fn accepted(c: &Connection) -> rusqlite::Result<()> {
    insert_accepted(c, OWNER, BOB, "Bob", 1).map(drop)
}

fn removing(c: &Connection) -> rusqlite::Result<()> {
    insert_accepted(c, OWNER, BOB, "Bob", 1)?;
    set_state(c, OWNER, BOB, FriendshipState::Removing)
}

fn store_session(c: &Connection) -> rusqlite::Result<()> {
    c.execute(
        "INSERT INTO ratchet_sessions (owner_key, peer_identity, peer_device, record, updated_at) \
         VALUES (?1, ?2, 'd', x'00', 1)",
        params![OWNER.to_ascii_lowercase(), BOB],
    )
    .map(drop)
}

fn state_of(conn: &Connection) -> Option<String> {
    conn.query_row(
        "SELECT friendship_state FROM friends WHERE owner_key = ?1 AND public_key = ?2",
        params![OWNER, BOB],
        |row| row.get(0),
    )
    .ok()
}

fn fail_on(conn: &Connection, table: &str) {
    conn.execute_batch(&format!(
        "CREATE TEMP TRIGGER injected BEFORE INSERT ON {table} \
         BEGIN SELECT RAISE(ABORT, 'injected'); END;"
    ))
    .unwrap();
}

#[test]
fn a_failed_block_keeps_the_friend() {
    let conn = conn();
    insert_accepted(&conn, OWNER, BOB, "Bob", 1).unwrap();
    queue_both(&conn);
    fail_on(&conn, "blocked_users");
    assert!(block(&conn, OWNER, BOB, "Bob", 2).is_err());
    assert_eq!(state_of(&conn).as_deref(), Some("accepted"));
    assert_eq!(
        left(&conn),
        (1, 1),
        "a failed block keeps the rows it would end"
    );
    conn.execute_batch("DROP TRIGGER injected").unwrap();
    block(&conn, OWNER, BOB, "Bob", 2).unwrap();
    assert_eq!(state_of(&conn), None);
    assert!(blocked::is_blocked(&conn, OWNER, BOB).unwrap());
}

#[test]
fn a_failed_removal_leaves_the_friend_accepted() {
    let conn = conn();
    insert_accepted(&conn, OWNER, BOB, "Bob", 1).unwrap();
    let stream = format!("friend_handshake:{BOB}");
    let row = outbox::NewRow {
        owner_key: OWNER,
        stream: &stream,
        client_id: "unfriended:bb",
        kind: "friend_handshake",
        payload: b"{}",
        next_at: 2,
        deadline: 9,
    };
    fail_on(&conn, "outbox");
    assert!(mark_removing(&conn, OWNER, BOB, &row).is_err());
    assert_eq!(state_of(&conn).as_deref(), Some("accepted"));
    conn.execute_batch("DROP TRIGGER injected").unwrap();
    mark_removing(&conn, OWNER, BOB, &row).unwrap();
    assert_eq!(state_of(&conn).as_deref(), Some("removing"));
    assert_eq!(outbox::unsettled_count(&conn, OWNER).unwrap(), 1);
}

#[test]
fn a_failed_accept_keeps_the_request_and_the_pending_friend() {
    let conn = conn();
    super::super::insert_pending_out(&conn, OWNER, BOB, "Bob", 1).unwrap();
    conn.execute_batch(
        "CREATE TEMP TRIGGER injected BEFORE UPDATE ON friends \
         WHEN NEW.dht_record_key IS NOT NULL BEGIN SELECT RAISE(ABORT, 'injected'); END;",
    )
    .unwrap();
    let keys = RecordKeys {
        profile: Some("profile"),
        mailbox: None,
    };
    assert!(accept_with_records(&conn, OWNER, BOB, Some("Bobby"), keys).is_err());
    assert_eq!(state_of(&conn).as_deref(), Some("pending_out"));
}

/// One `dm` and one `friend_handshake` row queued to Bob.
fn queue_both(c: &Connection) {
    outbox::queue_for_test(c, OWNER, &format!("dm:{BOB}"), "dm-1", b"m");
    outbox::queue_for_test(c, OWNER, &format!("friend_handshake:{BOB}"), "hs-1", b"h");
}

/// How many rows to Bob each stream holds: `(dm, friend_handshake)`.
fn left(c: &Connection) -> (usize, usize) {
    let count = |kind: &str| {
        outbox::stream_rows(c, OWNER, &format!("{kind}:{BOB}"))
            .unwrap()
            .len()
    };
    (count("dm"), count("friend_handshake"))
}

/// Review RV5 item 3: every friend change that ends a friendship (or
/// starts a new one over an ended one) fails the rows it ends in its own
/// transaction, so none is left live by a crash after the commit, and
/// returns them for the echo.
#[test]
fn every_ending_change_fails_its_rows_in_its_own_commit() {
    type Change = fn(&Connection) -> rusqlite::Result<Vec<Row>>;
    type Setup = fn(&Connection) -> rusqlite::Result<()>;
    let unfriended_row = |c: &Connection| {
        let stream = format!("friend_handshake:{BOB}");
        let row = outbox::NewRow {
            owner_key: OWNER,
            stream: &stream,
            client_id: "unfriended:bb",
            kind: "friend_handshake",
            payload: b"{}",
            next_at: 0,
            deadline: i64::MAX,
        };
        mark_removing(c, OWNER, BOB, &row)
    };
    let none: Setup = |_| Ok(());
    // (name, setup, change, the rows to Bob left after it: (dm, handshake))
    let cases: [(&str, Setup, Change, (usize, usize)); 13] = [
        (
            "cancel_request",
            pending,
            |c| cancel_request(c, OWNER, BOB),
            (0, 0),
        ),
        (
            "block",
            accepted,
            |c| block(c, OWNER, BOB, "Bob", 2),
            (0, 0),
        ),
        (
            "unfriended",
            accepted,
            |c| unfriended(c, OWNER, BOB),
            (0, 0),
        ),
        (
            "expiry",
            pending,
            |c| {
                let expired = super::super::delete_pending_out_before(c, OWNER, 5)?;
                Ok(expired.into_iter().flat_map(|(_, rows)| rows).collect())
            },
            (0, 0),
        ),
        (
            "accept_request",
            none,
            |c| accept_request(c, OWNER, BOB, "Bob", 2, RecordKeys::default()),
            (1, 0),
        ),
        (
            "replace_with_invited",
            removing,
            |c| replace_with_invited(c, OWNER, BOB, "Bob", 2, "p", "m"),
            (0, 0),
        ),
        (
            "insert_pending_out",
            none,
            |c| super::super::insert_pending_out(c, OWNER, BOB, "Bob", 2),
            (1, 0),
        ),
        (
            "insert_accepted",
            removing,
            |c| insert_accepted(c, OWNER, BOB, "Bob", 2),
            (0, 0),
        ),
        (
            "add_by_code",
            none,
            |c| add_by_code(c, OWNER, BOB, "Bob", 2, "p"),
            (1, 0),
        ),
        (
            "reject_request",
            none,
            |c| reject_request(c, OWNER, BOB),
            (0, 1),
        ),
        ("forget", accepted, |c| forget(c, OWNER, BOB), (0, 1)),
        ("mark_removing", accepted, unfriended_row, (0, 1)),
        (
            "forget_removed",
            removing,
            |c| {
                for id in ["dm-1", "hs-1"] {
                    outbox::settle(c, OWNER, id, rekindle_types::message::SendState::Failed)?;
                }
                forget_removed(c, OWNER).map(|_| Vec::new())
            },
            (0, 0),
        ),
    ];
    for (name, setup, change, after) in cases {
        let conn = conn();
        setup(&conn).unwrap();
        queue_both(&conn);
        let failed = change(&conn).unwrap();
        assert_eq!(left(&conn), after, "{name}: the rows it ends are gone");
        // A re-added removing friend's rows all went with the removal.
        // A removal fails both rows and leaves only its own `Unfriended`.
        let ended = if name == "mark_removing" {
            2
        } else if name == "insert_accepted" || name == "replace_with_invited" {
            1
        } else {
            2 - after.0.min(1) - after.1.min(1)
        };
        if name != "forget_removed" {
            assert_eq!(failed.len(), ended, "{name}: the failed rows are returned");
        }
    }
}

/// Review RV5 item 5: a friend-list change and its `dht_write` row commit
/// together, so no crash between them loses the list's write: a failed row
/// keeps the change out too, and a later change leaves one row.
#[test]
fn a_friend_list_change_and_its_row_commit_together() {
    let conn = conn();
    let list = |at| outbox::ListWrite {
        at,
        due: at,
        deadline: i64::MAX,
    };
    let rows = |c: &Connection| {
        outbox::stream_rows(c, OWNER, outbox::FRIEND_LIST_STREAM)
            .unwrap()
            .len()
    };
    fail_on(&conn, "outbox");
    assert!(changing_list(&conn, OWNER, list(1), accepted).is_err());
    assert_eq!(state_of(&conn), None, "the change went with its row");
    conn.execute_batch("DROP TRIGGER injected").unwrap();
    changing_list(&conn, OWNER, list(1), accepted).unwrap();
    assert_eq!(
        (state_of(&conn).as_deref(), rows(&conn)),
        (Some("accepted"), 1)
    );
    changing_list(&conn, OWNER, list(2), |c| block(c, OWNER, BOB, "Bob", 3)).unwrap();
    assert_eq!(rows(&conn), 1, "the later change superseded the row");
    // The expiry tick that expires nothing queues nothing.
    conn.execute("DELETE FROM outbox", []).unwrap();
    expire_pending_out(&conn, OWNER, 5, list(3)).unwrap();
    assert_eq!(rows(&conn), 0);
}

/// Review RV5 (coordinator): a change that starts or continues a
/// friendship never fails the rows that friendship committed.
#[test]
fn a_continuing_friendship_keeps_its_rows() {
    for setup in [pending, accepted] {
        let conn = conn();
        setup(&conn).unwrap();
        queue_both(&conn);
        assert!(
            super::super::insert_pending_out(&conn, OWNER, BOB, "Bob", 2)
                .unwrap()
                .is_empty()
        );
        assert!(insert_accepted(&conn, OWNER, BOB, "Bob", 2)
            .unwrap()
            .is_empty());
        assert!(
            accept_request(&conn, OWNER, BOB, "Bob", 2, RecordKeys::default())
                .unwrap()
                .is_empty()
        );
        assert_eq!(left(&conn), (1, 1));
    }
}
