use super::*;
use crate::repo::fixture;

const OWNER: [u8; 32] = [1; 32];
const PEER: [u8; 32] = [2; 32];

fn due(id: u8) -> Due {
    Due {
        msg_id: [id; 32],
        observed_ratchet_key: [9; 32],
    }
}

#[test]
fn nothing_goes_before_a_failure_is_noted() {
    let conn = fixture::identity_conn(&OWNER);
    assert!(take_due(&conn, &OWNER, &PEER, 0).unwrap().due.is_empty());
    note_failed(&conn, &OWNER, &PEER, &due(1), 0).unwrap();
    assert_eq!(take_due(&conn, &OWNER, &PEER, 1).unwrap().due, vec![due(1)]);
    // Sent: not again until it fails again.
    assert!(take_due(&conn, &OWNER, &PEER, 2).unwrap().due.is_empty());
}

#[test]
fn five_requests_per_message() {
    let conn = fixture::identity_conn(&OWNER);
    let window = ms(PEER_WINDOW);
    let mut sent = 0;
    for i in 0..8 {
        // Outside each other's window, so only the message cap binds.
        let now = i * window * 2;
        note_failed(&conn, &OWNER, &PEER, &due(1), now).unwrap();
        sent += take_due(&conn, &OWNER, &PEER, now).unwrap().due.len();
    }
    assert_eq!(sent, MAX_PER_MESSAGE as usize);
}

#[test]
fn a_message_failing_after_its_last_request_is_exhausted_once() {
    let conn = fixture::identity_conn(&OWNER);
    let window = ms(PEER_WINDOW);
    let mut exhausted = 0;
    for i in 0..=i64::from(MAX_PER_MESSAGE) {
        let now = i * window * 2;
        note_failed(&conn, &OWNER, &PEER, &due(1), now).unwrap();
        let taken = take_due(&conn, &OWNER, &PEER, now).unwrap();
        exhausted += taken.exhausted;
        assert_eq!(taken.due.is_empty(), i == i64::from(MAX_PER_MESSAGE));
    }
    assert_eq!(exhausted, 1);
    // Not counted again until it fails again.
    assert_eq!(
        take_due(&conn, &OWNER, &PEER, window * 20)
            .unwrap()
            .exhausted,
        0
    );
}

#[test]
fn ten_requests_per_peer_per_three_hours() {
    let conn = fixture::identity_conn(&OWNER);
    for id in 0..12 {
        note_failed(&conn, &OWNER, &PEER, &due(id), i64::from(id)).unwrap();
    }
    assert_eq!(take_due(&conn, &OWNER, &PEER, 100).unwrap().due.len(), 10);
    assert!(take_due(&conn, &OWNER, &PEER, 200).unwrap().due.is_empty());
    // Another peer has its own window.
    note_failed(&conn, &OWNER, &[3; 32], &due(1), 0).unwrap();
    assert_eq!(take_due(&conn, &OWNER, &[3; 32], 300).unwrap().due.len(), 1);
    // Past the window the rest go.
    let later = 100 + ms(PEER_WINDOW);
    assert_eq!(
        take_due(&conn, &OWNER, &PEER, later).unwrap().due,
        vec![due(10), due(11)]
    );
    prune(&conn, &OWNER, later + ms(PEER_WINDOW)).unwrap();
    let sends: u32 = conn
        .query_row("SELECT COUNT(*) FROM retry_request_sends", [], |r| r.get(0))
        .unwrap();
    assert_eq!(sends, 0);
}
