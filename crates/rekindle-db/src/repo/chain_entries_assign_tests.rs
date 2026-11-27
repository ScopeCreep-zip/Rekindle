use super::*;
use crate::repo::fixture::{self, OWNER};

fn friend(conn: &Connection) {
    conn.execute(
        "INSERT INTO friends (owner_key, public_key, added_at, friendship_state) \
             VALUES (?1, 'bob', 0, 'accepted')",
        params![OWNER],
    )
    .unwrap();
}

/// Seqs follow the persisted head, and a re-assignment (a re-hint, a
/// pass after a crash) reuses the recorded seq; an unsealed seq steps
/// the head back, and an unlanded entry is never retired.
#[test]
fn a_row_keeps_its_chain_seq_and_the_next_follows_the_head() {
    let conn = fixture::conn();
    friend(&conn);
    assert_eq!(assign(&conn, OWNER, "bob", 4).unwrap(), Some(1));
    assert_eq!(assign(&conn, OWNER, "bob", 5).unwrap(), Some(2));
    assert_eq!(assign(&conn, OWNER, "bob", 4).unwrap(), Some(1));
    assert_eq!(assigned(&conn, OWNER, "bob", 5).unwrap(), Some(2));
    assert!(unassign(&conn, OWNER, "bob", 2).unwrap());
    assert_eq!(assign(&conn, OWNER, "bob", 5).unwrap(), Some(2));
    assert_eq!(retirable(&conn, OWNER, "bob", i64::MAX).unwrap(), None);
    assert_eq!(assign(&conn, OWNER, "eve", 1).unwrap(), None);
}

/// A hinted row that fails at its deadline (its outbox row gone) keeps
/// its sealed entry pending until it lands, and the next row takes the
/// following seq: a sealed seq is never released.
#[test]
fn a_hinted_row_that_fails_still_lands_and_the_next_takes_the_following_seq() {
    let conn = fixture::conn();
    friend(&conn);
    assert_eq!(assign(&conn, OWNER, "bob", 4).unwrap(), Some(1));
    set_frame(&conn, OWNER, "bob", 1, b"hint").unwrap();
    // Row 4 failed at its deadline: it is not in the outbox.
    assert_eq!(assign(&conn, OWNER, "bob", 5).unwrap(), Some(2));
    assert!(!unassign(&conn, OWNER, "bob", 1).unwrap());
    assert_eq!(
        pending(&conn, OWNER, "bob").unwrap(),
        [(1, 4, b"hint".to_vec())]
    );
    assert_eq!(landed(&conn, OWNER, "bob", (1, 1), 10).unwrap(), 1);
    assert!(has_landed(&conn, OWNER, "bob", 4).unwrap());
    assert!(pending(&conn, OWNER, "bob").unwrap().is_empty());
}

/// A seq is sealed once: a re-hint's fresh framing never replaces the
/// bytes an append may already carry (review LV22 H1), so the chain
/// gets the first hint's frame.
#[test]
fn a_sealed_entry_keeps_its_first_frame() {
    let conn = fixture::conn();
    friend(&conn);
    assert_eq!(assign(&conn, OWNER, "bob", 4).unwrap(), Some(1));
    assert_eq!(
        set_frame(&conn, OWNER, "bob", 1, b"first").unwrap(),
        Some(b"first".to_vec())
    );
    assert_eq!(
        set_frame(&conn, OWNER, "bob", 1, b"again").unwrap(),
        Some(b"first".to_vec())
    );
    assert_eq!(
        pending(&conn, OWNER, "bob").unwrap(),
        [(1, 4, b"first".to_vec())]
    );
}
