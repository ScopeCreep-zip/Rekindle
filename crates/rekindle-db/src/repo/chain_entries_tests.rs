use super::*;
use crate::repo::fixture::{self, OWNER};

fn outbox_row(conn: &Connection, stream_seq: i64) {
    conn.execute(
        "INSERT INTO outbox (owner_key, stream, stream_seq, client_id, kind, payload, \
             state, next_at, deadline) VALUES (?1, 'dm:bob', ?2, ?3, 'dm', x'00', 'sent', 0, 0)",
        params![OWNER, stream_seq, format!("c{stream_seq}")],
    )
    .unwrap();
}

/// Only acknowledged entries older than the cutoff retire, and an
/// unacknowledged one holds back every entry after it.
#[test]
fn the_retire_point_stops_at_the_horizon_and_the_first_unacknowledged_entry() {
    let conn = fixture::conn();
    for (chain, stream, at) in [(1, 1, 10), (2, 2, 20), (3, 3, 30), (4, 4, 40), (5, 5, 90)] {
        record(&conn, OWNER, "bob", chain, stream, at).unwrap();
    }
    assert_eq!(retirable(&conn, OWNER, "bob", 50).unwrap(), Some(4));
    assert_eq!(retirable(&conn, OWNER, "bob", 25).unwrap(), Some(2));
    outbox_row(&conn, 3);
    assert_eq!(retirable(&conn, OWNER, "bob", 50).unwrap(), Some(2));
    // Failed at its deadline, the row stays for a resend and no longer
    // holds the chain (live A/B finding 73, dm_hint_loss gap scenarios).
    conn.execute(
        "UPDATE outbox SET state = 'failed' WHERE stream_seq = 3",
        [],
    )
    .unwrap();
    assert_eq!(retirable(&conn, OWNER, "bob", 50).unwrap(), Some(4));
    conn.execute("UPDATE outbox SET state = 'sent' WHERE stream_seq = 3", [])
        .unwrap();
    assert_eq!(retirable(&conn, OWNER, "bob", 5).unwrap(), None);
    assert_eq!(forget_through(&conn, OWNER, "bob", 2).unwrap(), 2);
    assert_eq!(retirable(&conn, OWNER, "bob", 50).unwrap(), None);
    assert_eq!(retirable(&conn, OWNER, "eve", 50).unwrap(), None);
}

/// A group chain retires by age alone, to the newest entry older than
/// the cutoff.
#[test]
fn a_group_chain_retires_by_age_only() {
    let conn = fixture::conn();
    let stream = "group_dm:00";
    for (chain, stream_seq, at) in [(1, 1, 10), (2, 2, 20), (3, 3, 90)] {
        record(&conn, OWNER, stream, chain, stream_seq, at).unwrap();
    }
    assert_eq!(retirable_by_age(&conn, OWNER, stream, 50).unwrap(), Some(2));
    assert_eq!(retirable_by_age(&conn, OWNER, stream, 5).unwrap(), None);
    assert_eq!(forget_through(&conn, OWNER, stream, 2).unwrap(), 2);
    assert_eq!(retirable_by_age(&conn, OWNER, stream, 50).unwrap(), None);
}
