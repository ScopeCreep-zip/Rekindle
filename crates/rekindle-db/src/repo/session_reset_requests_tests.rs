use super::*;
use crate::repo::fixture::{conn, OWNER};

#[test]
fn a_request_is_taken_once() {
    let conn = conn();
    insert(&conn, OWNER, "bb", 10).unwrap();
    assert!(take(&conn, OWNER, "bb").unwrap());
    assert!(!take(&conn, OWNER, "bb").unwrap(), "consumed");
    assert!(!take(&conn, OWNER, "cc").unwrap(), "never asked");
}

#[test]
fn requests_past_the_lifetime_expire() {
    let conn = conn();
    insert(&conn, OWNER, "old", 10).unwrap();
    insert(&conn, OWNER, "new", 100).unwrap();
    assert_eq!(delete_before(&conn, OWNER, 50).unwrap(), 1);
    assert!(!take(&conn, OWNER, "old").unwrap());
    assert!(take(&conn, OWNER, "new").unwrap());
}
