use super::*;
use crate::repo::fixture::{self, OWNER};

fn incoming(public_key: &str, received_at: i64) -> Incoming<'_> {
    Incoming {
        public_key,
        display_name: "Bob",
        message: "hi",
        received_at,
        profile_dht_key: "profile",
        route_blob: &[1, 2],
        mailbox_dht_key: "mailbox",
        invite_id: Some("invite"),
        reply_inbox_key: "VLD0:reply",
        reply_writer: "VLD0:pub:sec",
    }
}

#[test]
fn the_same_request_again_changes_nothing() {
    let conn = fixture::conn();
    assert!(upsert(&conn, OWNER, &incoming("bob", 5)).unwrap());
    assert!(!upsert(&conn, OWNER, &incoming("bob", 9)).unwrap());
    assert_eq!(list(&conn, OWNER).unwrap()[0].received_at, 5);
    let changed = Incoming {
        message: "hello again",
        ..incoming("bob", 9)
    };
    assert!(upsert(&conn, OWNER, &changed).unwrap());
    assert_eq!(list(&conn, OWNER).unwrap().len(), 1);
}

#[test]
fn a_stored_request_lists_and_answers() {
    let conn = fixture::conn();
    upsert(&conn, OWNER, &incoming("bob", 5)).unwrap();
    assert_eq!(
        list(&conn, OWNER).unwrap(),
        [PendingFriendRequest {
            public_key: "bob".into(),
            display_name: "Bob".into(),
            message: "hi".into(),
            received_at: 5,
        }]
    );
    let a = answer(&conn, OWNER, "bob").unwrap().expect("bob asked");
    assert_eq!(a.route_blob.as_deref(), Some(&[1, 2][..]));
    assert_eq!(a.invite_id.as_deref(), Some("invite"));
    assert_eq!(
        answer(&conn, OWNER, "ghost").unwrap(),
        None,
        "no request, no answer"
    );
}

#[test]
fn a_second_request_replaces_the_first_and_old_ones_expire() {
    let conn = fixture::conn();
    upsert(&conn, OWNER, &incoming("bob", 5)).unwrap();
    let second = Incoming {
        message: "hi again",
        ..incoming("bob", 9)
    };
    assert!(upsert(&conn, OWNER, &second).unwrap());
    upsert(&conn, OWNER, &incoming("carol", 1)).unwrap();
    assert_eq!(delete_received_before(&conn, OWNER, 6).unwrap(), 1);
    let left = list(&conn, OWNER).unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(
        (left[0].public_key.as_str(), left[0].received_at),
        ("bob", 9)
    );
    delete(&conn, OWNER, "bob").unwrap();
    assert!(list(&conn, OWNER).unwrap().is_empty());
}
