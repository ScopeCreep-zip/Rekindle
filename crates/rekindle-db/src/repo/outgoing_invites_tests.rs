use super::*;
use crate::repo::fixture::{self, OWNER};

#[test]
fn an_invite_moves_through_its_states() {
    let conn = fixture::conn();
    insert(&conn, OWNER, "a", "rekindle://a", "inbox-a", 1).unwrap();
    insert(&conn, OWNER, "b", "rekindle://b", "inbox-b", 2).unwrap();
    mark_responded(&conn, OWNER, "a", "bob").unwrap();
    let open_now = open(&conn, OWNER).unwrap();
    assert_eq!(open_now.len(), 2);
    assert_eq!(open_now[1].accepted_by.as_deref(), Some("bob"));
    mark_accepted(&conn, OWNER, "a").unwrap();
    assert!(cancel(&conn, OWNER, "b").unwrap());
    assert!(!admits_request(&conn, OWNER, "b", "x", 0).unwrap());
    assert!(open(&conn, OWNER).unwrap().is_empty());
    insert(&conn, OWNER, "c", "rekindle://c", "inbox-c", 0).unwrap();
    assert_eq!(
        pending_inboxes(&conn, OWNER).unwrap(),
        vec!["inbox-c".to_string()]
    );
    assert_eq!(
        expire(&conn, OWNER, INVITE_EXPIRY_MS + 1).unwrap(),
        vec!["inbox-c".to_string()]
    );
    assert!(pending_inboxes(&conn, OWNER).unwrap().is_empty());
    assert_eq!(
        inbox_of(&conn, OWNER, "c").unwrap().as_deref(),
        Some("inbox-c")
    );
}

#[test]
fn only_an_open_unexpired_invite_or_its_own_responder_is_admitted() {
    let conn = fixture::conn();
    let expires = insert(&conn, OWNER, "i", "u", "inbox-i", 0).unwrap();
    assert!(admits_request(&conn, OWNER, "i", "bob", expires).unwrap());
    assert!(!admits_request(&conn, OWNER, "i", "bob", expires + 1).unwrap());
    assert!(!admits_request(&conn, OWNER, "unknown", "bob", 0).unwrap());
    mark_responded(&conn, OWNER, "i", "bob").unwrap();
    assert!(admits_request(&conn, OWNER, "i", "bob", 0).unwrap());
    assert!(!admits_request(&conn, OWNER, "i", "carol", 0).unwrap());
    insert(&conn, OWNER, "e", "u", "inbox-e", 0).unwrap();
    expire(&conn, OWNER, expires + 1).unwrap();
    assert!(!admits_request(&conn, OWNER, "e", "bob", 0).unwrap());
}
