use super::*;
use crate::repo::fixture::{self, OWNER};

fn invite_at(at: i64) -> CreatorInviteRow {
    CreatorInviteRow {
        anchor: "VLD0:anchor".into(),
        inviter_key: "a".repeat(64),
        community_name: "LAN".into(),
        received_at: at,
    }
}

/// Decision (20): an invite survives until taken, only by its inviter.
#[test]
fn an_invite_is_held_and_taken_only_by_its_inviter() {
    let conn = fixture::conn();
    put_invite(&conn, OWNER, &invite_at(5)).unwrap();
    put_invite(&conn, OWNER, &invite_at(6)).unwrap();
    assert_eq!(invites(&conn, OWNER).unwrap(), vec![invite_at(6)]);
    assert_eq!(
        take_invite(&conn, OWNER, "VLD0:anchor", &"b".repeat(64)).unwrap(),
        None
    );
    assert_eq!(
        take_invite(&conn, OWNER, "VLD0:anchor", &"a".repeat(64)).unwrap(),
        Some(invite_at(6))
    );
    assert!(invites(&conn, OWNER).unwrap().is_empty());
    put_invite(&conn, OWNER, &invite_at(7)).unwrap();
    let a = "a".repeat(64);
    assert_eq!(
        take_accepted(&conn, OWNER, "VLD0:anchor", &a).unwrap(),
        None,
        "not accepted"
    );
    assert!(mark_accepted(&conn, OWNER, "VLD0:anchor", &a).unwrap());
    assert_eq!(
        take_accepted(&conn, OWNER, "VLD0:anchor", &a).unwrap(),
        Some(invite_at(7))
    );
    let mut bad = invite_at(5);
    bad.inviter_key = "short".into();
    assert!(put_invite(&conn, OWNER, &bad).is_err());
}

/// Decision (20): the sweep takes every waiting co-creator once.
#[test]
fn the_sweep_takes_every_pending_co_creator_once() {
    let conn = fixture::conn();
    let (b, c) = ("b".repeat(64), "c".repeat(64));
    put_pending(&conn, OWNER, "VLD0:x", &[b.clone(), c.clone()]).unwrap();
    assert!(remove_pending(&conn, OWNER, "VLD0:x", &c).unwrap());
    assert!(!remove_pending(&conn, OWNER, "VLD0:x", &c).unwrap());
    put_pending(&conn, OWNER, "VLD0:y", std::slice::from_ref(&c)).unwrap();
    clear_pending(&conn, OWNER, "VLD0:y").unwrap();
    assert_eq!(
        take_all_pending(&conn, OWNER).unwrap(),
        vec![("VLD0:x".to_string(), b)]
    );
    assert!(take_all_pending(&conn, OWNER).unwrap().is_empty());
}
