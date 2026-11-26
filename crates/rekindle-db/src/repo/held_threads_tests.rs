use super::*;
use crate::repo::fixture::{self, OWNER};

#[test]
fn a_hold_is_kept_once_and_cascades_with_its_community() {
    let conn = fixture::conn();
    fixture::community(&conn, "c");
    let id = "0a".repeat(16);
    hold(&conn, OWNER, "c", &[id.clone(), id.clone()], 5).unwrap();
    hold(&conn, OWNER, "c", std::slice::from_ref(&id), 9).unwrap();
    assert_eq!(load(&conn, OWNER).unwrap(), vec![("c".to_owned(), id)]);
    conn.execute("DELETE FROM communities WHERE id = 'c'", [])
        .unwrap();
    assert!(load(&conn, OWNER).unwrap().is_empty());
}
