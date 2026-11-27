use super::*;
use crate::repo::fixture;

const OWNER: [u8; 32] = [1; 32];
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

fn key(n: u32) -> UnprocessedKey {
    UnprocessedKey {
        peer_identity: [2; 32],
        header_dh: [3; 32],
        header_n: n,
    }
}

/// D24 / R3-X-1: every retention bound on the delivery path derives
/// from `DELIVERY_HORIZON` and outlasts it — a message that arrives at
/// the end of the horizon still finds its prekey (grace ≥ inbox
/// retention) and still has time to apply (unprocessed age > horizon).
#[test]
fn the_retention_bounds_outlast_the_horizon() {
    use rekindle_crypto::signal::prekey_record::RETIRED_ARCHIVE_MS;
    let horizon_ms = i64::try_from(DELIVERY_HORIZON.as_millis()).unwrap();
    assert!(RETIRED_ARCHIVE_MS >= horizon_ms);
    assert!(UNPROCESSED_MAX_AGE > DELIVERY_HORIZON);
    assert_eq!(UNPROCESSED_MAX_AGE, DELIVERY_HORIZON * 3 / 2);
    assert_eq!(UNPROCESSED_MAX_AGE.as_secs(), 45 * 24 * 60 * 60);
}

/// Audit D-9: a row that does not read back is set aside, and the
/// others still replay; the age bound sweeps it.
#[test]
fn a_corrupt_row_is_quarantined_and_the_rest_replay() {
    let conn = fixture::identity_conn(&OWNER);
    insert(&conn, &OWNER, &key(0), 1, b"good", 5).unwrap();
    conn.execute(
        "INSERT INTO unprocessed (owner_key, peer_identity, header_dh, header_n, type_id, \
             plaintext, received_at) VALUES (?1, 'not-a-key', zeroblob(32), 9, 1, x'00', 4)",
        params![identity_text(&OWNER)],
    )
    .unwrap();
    let replay = take_for_replay(&conn, &OWNER, 10).unwrap();
    assert_eq!(replay.iter().map(|r| r.key).collect::<Vec<_>>(), [key(0)]);
    let quarantined = |conn: &Connection| -> i64 {
        conn.query_row(
            "SELECT count(*) FROM unprocessed WHERE quarantined_at IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(quarantined(&conn), 1);
    assert_eq!(take_for_replay(&conn, &OWNER, 11).unwrap().len(), 1);
    let later = 10 + i64::try_from(UNPROCESSED_MAX_AGE.as_millis()).unwrap() + 1;
    delete(&conn, &OWNER, &key(0)).unwrap();
    assert!(take_for_replay(&conn, &OWNER, later).unwrap().is_empty());
    assert_eq!(quarantined(&conn), 0);
}

/// Audit D-9: the header's DH key is 32 bytes and the counters are
/// non-negative.
#[test]
fn malformed_rows_are_refused() {
    let conn = fixture::identity_conn(&OWNER);
    let owner = identity_text(&OWNER);
    let peer = identity_text(&[2; 32]);
    for (dh, n, attempts, at) in [
        ("zeroblob(31)", "0", "0", "0"),
        ("'0123456789abcdef0123456789abcdef'", "0", "0", "0"),
        ("zeroblob(32)", "-1", "0", "0"),
        ("zeroblob(32)", "4294967296", "0", "0"),
        ("zeroblob(32)", "0", "-1", "0"),
        ("zeroblob(32)", "0", "0", "-1"),
    ] {
        let sql = format!(
            "INSERT INTO unprocessed (owner_key, peer_identity, header_dh, header_n, \
                 type_id, plaintext, attempts, received_at) \
                 VALUES (?1, ?2, {dh}, {n}, 1, x'00', {attempts}, {at})"
        );
        assert!(conn.execute(&sql, params![owner, peer]).is_err(), "{sql}");
    }
}

#[test]
fn a_row_replays_until_applied() {
    let conn = fixture::identity_conn(&OWNER);
    insert(&conn, &OWNER, &key(0), 0x0100, b"hello", 5).unwrap();
    insert(&conn, &OWNER, &key(1), 0x0100, b"again", 6).unwrap();
    let replay = take_for_replay(&conn, &OWNER, 10).unwrap();
    assert_eq!(replay.len(), 2);
    assert_eq!(replay[0].key, key(0));
    assert_eq!(
        (replay[0].plaintext.as_slice(), replay[0].type_id),
        (&b"hello"[..], 0x0100)
    );
    assert_eq!(replay[0].attempts, 1);
    delete(&conn, &OWNER, &key(0)).unwrap();
    let replay = take_for_replay(&conn, &OWNER, 10).unwrap();
    assert_eq!(replay.len(), 1);
    assert_eq!((replay[0].key, replay[0].attempts), (key(1), 2));
}

#[test]
fn a_header_is_recorded_once() {
    let conn = fixture::identity_conn(&OWNER);
    insert(&conn, &OWNER, &key(0), 1, b"a", 0).unwrap();
    assert!(insert(&conn, &OWNER, &key(0), 1, b"a", 0).is_err());
}

#[test]
fn stale_and_exhausted_rows_are_dropped_at_replay() {
    let conn = fixture::identity_conn(&OWNER);
    let now = 100 * DAY_MS;
    insert(&conn, &OWNER, &key(0), 1, b"old", now - 46 * DAY_MS).unwrap();
    insert(&conn, &OWNER, &key(1), 1, b"recent", now - 44 * DAY_MS).unwrap();
    let replay = take_for_replay(&conn, &OWNER, now).unwrap();
    assert_eq!(replay.iter().map(|r| r.key).collect::<Vec<_>>(), [key(1)]);
    for _ in 1..MAX_UNPROCESSED_ATTEMPTS {
        assert_eq!(take_for_replay(&conn, &OWNER, now).unwrap().len(), 1);
    }
    assert!(take_for_replay(&conn, &OWNER, now).unwrap().is_empty());
}
