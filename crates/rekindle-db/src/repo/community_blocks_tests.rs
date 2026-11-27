use super::*;
use crate::repo::fixture::{self, OWNER};

fn pseudonyms(conn: &Connection, community: &str) -> Vec<String> {
    list(conn, OWNER, community)
        .unwrap()
        .into_iter()
        .map(|b| b.pseudonym)
        .collect()
}

#[test]
fn a_block_is_per_community_and_listed_until_lifted() {
    let conn = fixture::conn();
    fixture::community(&conn, "c");
    fixture::community(&conn, "d");
    insert(&conn, OWNER, "c", "EVE", 5).unwrap();
    insert(&conn, OWNER, "c", "mallory", 9).unwrap();
    assert_eq!(pseudonyms(&conn, "c"), ["mallory", "eve"]);
    assert!(pseudonyms(&conn, "d").is_empty(), "another community");
    assert!(delete(&conn, OWNER, "c", "Eve").unwrap());
    assert!(!delete(&conn, OWNER, "c", "eve").unwrap());
    assert_eq!(pseudonyms(&conn, "c"), ["mallory"]);
}

#[test]
fn leaving_the_community_drops_its_blocks() {
    let conn = fixture::conn();
    fixture::community(&conn, "c");
    insert(&conn, OWNER, "c", "eve", 1).unwrap();
    conn.execute("DELETE FROM communities WHERE id = 'c'", [])
        .unwrap();
    assert!(pseudonyms(&conn, "c").is_empty());
}
