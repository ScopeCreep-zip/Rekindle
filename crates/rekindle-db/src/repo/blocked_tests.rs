use super::*;
use crate::repo::fixture::{self, OWNER};

#[test]
fn a_block_is_listed_until_it_is_lifted() {
    let conn = fixture::conn();
    assert!(!is_blocked(&conn, OWNER, "eve").unwrap());
    insert(&conn, OWNER, "eve", "Eve", 5).unwrap();
    insert(&conn, OWNER, "mallory", "Mallory", 9).unwrap();
    assert!(is_blocked(&conn, OWNER, "eve").unwrap());
    let listed: Vec<String> = list(&conn, OWNER)
        .unwrap()
        .into_iter()
        .map(|b| b.public_key)
        .collect();
    assert_eq!(listed, ["mallory", "eve"]);
    assert!(delete(&conn, OWNER, "eve").unwrap());
    assert!(!delete(&conn, OWNER, "eve").unwrap());
    assert!(!is_blocked(&conn, OWNER, "eve").unwrap());
}
