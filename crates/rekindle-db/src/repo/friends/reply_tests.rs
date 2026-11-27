use super::*;
use crate::repo::fixture::{self, OWNER};
use crate::repo::friends::{insert_accepted, insert_pending_out, set_state};
use rekindle_types::friend::FriendshipState;

#[test]
fn only_open_requests_hold_their_reply_inbox() {
    let conn = fixture::conn();
    insert_pending_out(&conn, OWNER, "ada", "Ada", 1).unwrap();
    insert_pending_out(&conn, OWNER, "bo", "Bo", 1).unwrap();
    set_reply_inbox(&conn, OWNER, "ada", "VLD0:a").unwrap();
    set_reply_inbox(&conn, OWNER, "bo", "VLD0:b").unwrap();
    assert_eq!(reply_inboxes(&conn, OWNER).unwrap(), ["VLD0:a", "VLD0:b"]);
    set_state(&conn, OWNER, "bo", FriendshipState::Accepted).unwrap();
    assert_eq!(reply_inboxes(&conn, OWNER).unwrap(), ["VLD0:a"]);
    assert!(is_accepted_row(&conn, OWNER, "bo").unwrap());
    assert!(!is_accepted_row(&conn, OWNER, "ada").unwrap());
    assert!(!is_accepted_row(&conn, OWNER, "nobody").unwrap());
}

#[test]
fn a_peer_reply_is_taken_once() {
    let conn = fixture::conn();
    insert_accepted(&conn, OWNER, "cy", "Cy", 1).unwrap();
    assert_eq!(take_peer_reply(&conn, OWNER, "cy").unwrap(), None);
    set_peer_reply(&conn, OWNER, "cy", "VLD0:c", "VLD0:p:s").unwrap();
    assert_eq!(
        take_peer_reply(&conn, OWNER, "cy").unwrap(),
        Some(("VLD0:c".to_owned(), "VLD0:p:s".to_owned()))
    );
    assert_eq!(take_peer_reply(&conn, OWNER, "cy").unwrap(), None);
}
