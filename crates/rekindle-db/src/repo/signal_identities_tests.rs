use super::*;
use crate::repo::fixture;

const OWNER: [u8; 32] = [1; 32];
const OLD: [u8; 32] = [2; 32];
const NEW: [u8; 32] = [3; 32];

#[test]
fn first_contact_is_unverified_and_kept() {
    let conn = fixture::identity_conn(&OWNER);
    assert_eq!(get(&conn, &OWNER, &OLD).unwrap(), None);
    ensure(&conn, &OWNER, &OLD).unwrap();
    mark_verified(&conn, &OWNER, &OLD, 5).unwrap();
    ensure(&conn, &OWNER, &OLD).unwrap();
    let row = get(&conn, &OWNER, &OLD).unwrap().unwrap();
    assert_eq!(row.identity_key, OLD);
    assert!(row.verified, "a later contact keeps the row");
    assert_eq!(row.verified_at, Some(5));
}

#[test]
fn verification_is_marked_and_cleared() {
    let conn = fixture::identity_conn(&OWNER);
    assert!(!mark_verified(&conn, &OWNER, &OLD, 1).unwrap());
    ensure(&conn, &OWNER, &OLD).unwrap();
    assert!(mark_verified(&conn, &OWNER, &OLD, 1).unwrap());
    assert!(clear_verification(&conn, &OWNER, &OLD).unwrap());
    let row = get(&conn, &OWNER, &OLD).unwrap().unwrap();
    assert!(!row.verified);
    assert_eq!(row.verified_at, None);
}

#[test]
fn rotating_an_unverified_friend_records_no_change() {
    let conn = fixture::identity_conn(&OWNER);
    ensure(&conn, &OWNER, &OLD).unwrap();
    rotate(&conn, &OWNER, &OLD, &NEW, 9).unwrap();
    assert_eq!(get(&conn, &OWNER, &OLD).unwrap(), None);
    let row = get(&conn, &OWNER, &NEW).unwrap().unwrap();
    assert_eq!((row.identity_key, row.changed_from_key), (NEW, None));
    assert_eq!(verified_key_changed(&conn, &OWNER, &NEW).unwrap(), None);
}

#[test]
fn a_verified_key_change_escalates_until_acknowledged() {
    let conn = fixture::identity_conn(&OWNER);
    ensure(&conn, &OWNER, &OLD).unwrap();
    mark_verified(&conn, &OWNER, &OLD, 1).unwrap();
    rotate(&conn, &OWNER, &OLD, &NEW, 9).unwrap();

    let row = get(&conn, &OWNER, &NEW).unwrap().unwrap();
    assert!(!row.verified, "verification resets on a key change");
    assert_eq!((row.changed_from_key, row.changed_at), (Some(OLD), Some(9)));
    let Some(Escalation::VerifiedKeyChanged {
        old_safety_number,
        new_safety_number,
    }) = verified_key_changed(&conn, &OWNER, &NEW).unwrap()
    else {
        panic!("a verified key change escalates");
    };
    assert_eq!(old_safety_number, SafetyNumber::new(&OWNER, &OLD).digits());
    assert_eq!(new_safety_number, SafetyNumber::new(&OWNER, &NEW).digits());
    assert_ne!(old_safety_number, new_safety_number);

    acknowledge_key_change(&conn, &OWNER, &NEW).unwrap();
    assert_eq!(verified_key_changed(&conn, &OWNER, &NEW).unwrap(), None);
    assert!(!get(&conn, &OWNER, &NEW).unwrap().unwrap().verified);
}

#[test]
fn an_acknowledging_write_commits_with_its_acknowledgement_or_not_at_all() {
    let conn = fixture::identity_conn(&OWNER);
    ensure(&conn, &OWNER, &OLD).unwrap();
    mark_verified(&conn, &OWNER, &OLD, 1).unwrap();
    rotate(&conn, &OWNER, &OLD, &NEW, 9).unwrap();

    let failed: rusqlite::Result<()> = acknowledging(&conn, &OWNER, &NEW, |_| {
        Err(rusqlite::Error::QueryReturnedNoRows)
    });
    assert!(failed.is_err());
    assert!(
        verified_key_changed(&conn, &OWNER, &NEW).unwrap().is_some(),
        "a failed write acknowledges nothing"
    );

    assert_eq!(acknowledging(&conn, &OWNER, &NEW, |_| Ok(7)).unwrap(), 7);
    assert_eq!(verified_key_changed(&conn, &OWNER, &NEW).unwrap(), None);
    assert!(!get(&conn, &OWNER, &NEW).unwrap().unwrap().verified);
}

#[test]
fn a_second_rotation_keeps_the_last_verified_key_and_verifying_acknowledges() {
    let conn = fixture::identity_conn(&OWNER);
    let newer = [4u8; 32];
    ensure(&conn, &OWNER, &OLD).unwrap();
    mark_verified(&conn, &OWNER, &OLD, 1).unwrap();
    rotate(&conn, &OWNER, &OLD, &NEW, 9).unwrap();
    rotate(&conn, &OWNER, &NEW, &newer, 12).unwrap();
    let row = get(&conn, &OWNER, &newer).unwrap().unwrap();
    assert_eq!((row.changed_from_key, row.changed_at), (Some(OLD), Some(9)));

    mark_verified(&conn, &OWNER, &newer, 20).unwrap();
    assert_eq!(verified_key_changed(&conn, &OWNER, &newer).unwrap(), None);
    assert!(get(&conn, &OWNER, &newer).unwrap().unwrap().verified);
}
