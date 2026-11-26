use super::*;
use crate::repo::fixture::{self, OWNER};

#[test]
fn a_cursor_only_moves_up() {
    let conn = fixture::conn();
    assert_eq!(get(&conn, OWNER, "dm", "w").unwrap(), 0);
    assert!(advance(&conn, OWNER, "dm", "w", 3, 1).unwrap());
    assert!(!advance(&conn, OWNER, "dm", "w", 2, 2).unwrap());
    assert!(!advance(&conn, OWNER, "dm", "w", 3, 2).unwrap());
    assert_eq!(get(&conn, OWNER, "dm", "w").unwrap(), 3);
    assert_eq!(get(&conn, OWNER, "dm", "x").unwrap(), 0);
    reset(&conn, OWNER, "dm", "w").unwrap();
    assert_eq!(get(&conn, OWNER, "dm", "w").unwrap(), 0);
}

#[test]
fn a_friendship_that_ends_forgets_its_dm_cursor() {
    let conn = fixture::conn();
    advance(&conn, OWNER, "dm", "w", 3, 1).unwrap();
    advance(&conn, OWNER, "other", "w", 2, 1).unwrap();
    crate::repo::friends::forget(&conn, OWNER, "w").unwrap();
    assert_eq!(get(&conn, OWNER, "dm", "w").unwrap(), 0);
    assert_eq!(get(&conn, OWNER, "other", "w").unwrap(), 2);
}

/// G4: the cursor records its entry's stream seq; a cursor that
/// absorbed a remembered seq past the entry keeps the older one.
#[test]
fn a_cursor_records_its_entrys_stream_seq() {
    let conn = fixture::conn();
    let at = (OWNER, "group_dm:g", "w");
    assert_eq!(stream_seq(&conn, OWNER, at.1, at.2).unwrap(), 0);
    assert!(advance_entry(&conn, at, (1, 4), 1).unwrap());
    assert_eq!(stream_seq(&conn, OWNER, at.1, at.2).unwrap(), 4);
    step(&conn, OWNER, at.1, at.2, 3, 1).unwrap();
    assert!(advance_entry(&conn, at, (2, 5), 1).unwrap());
    assert_eq!(get(&conn, OWNER, at.1, at.2).unwrap(), 3);
    assert_eq!(stream_seq(&conn, OWNER, at.1, at.2).unwrap(), 4);
    assert!(!advance_entry(&conn, at, (3, 6), 2).unwrap());
    assert_eq!(stream_seq(&conn, OWNER, at.1, at.2).unwrap(), 6);
}

fn friend(conn: &Connection) {
    conn.execute(
        "INSERT INTO friends (owner_key, public_key, added_at, friendship_state) \
             VALUES (?1, 'bob', 0, 'accepted')",
        params![OWNER],
    )
    .unwrap();
}

/// Hints 1, 2 and 3 with the friendship's first `StreamRecord` at
/// either place (it can overtake hint 1 or come after hint 2, live A/B
/// findings 73 and 77): the cursor ends at 3 and no hint is held.
#[test]
fn a_first_stream_record_never_resets_the_hints_cursor() {
    for record_after in [1, 2] {
        let conn = fixture::conn();
        friend(&conn);
        for seq in 1..=3 {
            let at = step(&conn, OWNER, "dm", "bob", seq, 1).unwrap();
            assert_eq!(at, seq - 1, "hint {seq} is the cursor's next, none held");
            if seq == record_after {
                assert!(adopt_record(&conn, OWNER, "dm", "bob", "rec1").unwrap());
            }
        }
        assert_eq!(get(&conn, OWNER, "dm", "bob").unwrap(), 3);
    }
}

fn remembered(conn: &Connection, writer: &str) -> Vec<i64> {
    let mut stmt = conn
        .prepare("SELECT seq FROM stream_applied WHERE writer = ?1 ORDER BY seq")
        .unwrap();
    let rows = stmt.query_map([writer], |r| r.get(0)).unwrap();
    rows.collect::<rusqlite::Result<_>>().unwrap()
}

/// Hints 3, 1, 2 (3 overtook both): 3 is applied past the gap and
/// remembered, 1 steps the cursor, 2 closes the gap and the cursor
/// absorbs 3 (E2.5i R1-3: it stopped at 2, below a seq applied).
#[test]
fn a_closed_gap_absorbs_the_seqs_applied_past_it() {
    let conn = fixture::conn();
    assert_eq!(step(&conn, OWNER, "dm", "w", 3, 1).unwrap(), 0);
    assert_eq!(get(&conn, OWNER, "dm", "w").unwrap(), 0, "past a gap");
    assert_eq!(step(&conn, OWNER, "dm", "w", 1, 2).unwrap(), 0);
    assert_eq!(get(&conn, OWNER, "dm", "w").unwrap(), 1);
    assert_eq!(step(&conn, OWNER, "dm", "w", 2, 3).unwrap(), 1);
    assert_eq!(get(&conn, OWNER, "dm", "w").unwrap(), 3);
    assert!(remembered(&conn, "w").is_empty(), "passed seqs go");
    assert_eq!(get(&conn, OWNER, "dm", "x").unwrap(), 0, "per writer");
}

/// A duplicate at or below the cursor changes nothing and is never
/// remembered.
#[test]
fn a_duplicate_below_the_cursor_is_a_no_op() {
    let conn = fixture::conn();
    for seq in 1..=3 {
        step(&conn, OWNER, "dm", "w", seq, 1).unwrap();
    }
    assert_eq!(step(&conn, OWNER, "dm", "w", 2, 2).unwrap(), 3);
    assert_eq!(step(&conn, OWNER, "dm", "w", 3, 2).unwrap(), 3);
    assert_eq!(get(&conn, OWNER, "dm", "w").unwrap(), 3);
    assert!(remembered(&conn, "w").is_empty());
}

/// The remembered seqs are rows: a restart (a new connection on the
/// same database) still absorbs them, and the chain diff's `advance`
/// absorbs them as the hint's `step` does; a second gap stops it.
#[test]
fn a_restart_keeps_the_remembered_seqs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cursors.db");
    {
        let conn = fixture::conn_at(&path);
        for seq in [2, 3, 5] {
            step(&conn, OWNER, "dm", "w", seq, 1).unwrap();
        }
    }
    let conn = fixture::conn_at(&path);
    assert_eq!(get(&conn, OWNER, "dm", "w").unwrap(), 0);
    assert!(advance(&conn, OWNER, "dm", "w", 1, 2).unwrap());
    assert_eq!(get(&conn, OWNER, "dm", "w").unwrap(), 3, "4 is still a gap");
    assert_eq!(remembered(&conn, "w"), [5]);
    reset(&conn, OWNER, "dm", "w").unwrap();
    assert!(
        remembered(&conn, "w").is_empty(),
        "a new chain forgets them"
    );
}

/// A record that replaces the held one is a new chain: the cursor
/// restarts; the same key again changes nothing.
#[test]
fn a_replaced_stream_record_restarts_the_cursor() {
    let conn = fixture::conn();
    friend(&conn);
    assert!(adopt_record(&conn, OWNER, "dm", "bob", "rec1").unwrap());
    step(&conn, OWNER, "dm", "bob", 1, 1).unwrap();
    assert!(!adopt_record(&conn, OWNER, "dm", "bob", "rec1").unwrap());
    assert_eq!(get(&conn, OWNER, "dm", "bob").unwrap(), 1);
    assert!(adopt_record(&conn, OWNER, "dm", "bob", "rec2").unwrap());
    assert_eq!(get(&conn, OWNER, "dm", "bob").unwrap(), 0);
    assert_eq!(
        step(&conn, OWNER, "dm", "bob", 2, 1).unwrap(),
        0,
        "past a gap"
    );
    assert_eq!(get(&conn, OWNER, "dm", "bob").unwrap(), 0);
}
