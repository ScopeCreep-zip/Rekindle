use super::*;
use crate::repo::fixture;

const OWNER: [u8; 32] = [1; 32];
const PEER: [u8; 32] = [2; 32];
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

fn sent(id: u8, sent_at: i64) -> LoggedSend {
    LoggedSend {
        peer: PEER,
        msg_id: [id; 32],
        type_id: 0x0100,
        ratchet_key: [9; 32],
        plaintext: vec![id; 8],
        sent_at,
        client_id: None,
    }
}

#[test]
fn a_genuine_request_resends_once_and_archives() {
    let conn = fixture::identity_conn(&OWNER);
    insert(&conn, &OWNER, &sent(3, 10)).unwrap();
    let resend = take_for_resend(&conn, &OWNER, &PEER, &[3; 32], &[9; 32], 20).unwrap();
    assert_eq!(resend, Some(sent(3, 10)));
    // Archived: a copy of the request resends nothing.
    assert_eq!(get(&conn, &OWNER, &PEER, &[3; 32]).unwrap(), None);
    assert_eq!(
        take_for_resend(&conn, &OWNER, &PEER, &[3; 32], &[9; 32], 30).unwrap(),
        None
    );
}

#[test]
fn a_forged_key_changes_nothing() {
    let conn = fixture::identity_conn(&OWNER);
    insert(&conn, &OWNER, &sent(3, 10)).unwrap();
    assert_eq!(
        take_for_resend(&conn, &OWNER, &PEER, &[3; 32], &[8; 32], 20).unwrap(),
        None
    );
    assert_eq!(
        take_for_resend(&conn, &OWNER, &[7; 32], &[3; 32], &[9; 32], 20).unwrap(),
        None
    );
    // The row is untouched: the genuine request still finds it.
    assert_eq!(
        get(&conn, &OWNER, &PEER, &[3; 32]).unwrap(),
        Some(sent(3, 10))
    );
}

#[test]
fn rows_past_the_horizon_are_pruned() {
    let conn = fixture::identity_conn(&OWNER);
    let now = 40 * DAY_MS;
    insert(&conn, &OWNER, &sent(1, now - 31 * DAY_MS)).unwrap();
    insert(&conn, &OWNER, &sent(2, now - 29 * DAY_MS)).unwrap();
    assert_eq!(prune(&conn, &OWNER, now).unwrap(), 1);
    assert_eq!(get(&conn, &OWNER, &PEER, &[1; 32]).unwrap(), None);
    assert!(get(&conn, &OWNER, &PEER, &[2; 32]).unwrap().is_some());
    assert_eq!(SEND_LOG_MAX_AGE, DELIVERY_HORIZON);
}
