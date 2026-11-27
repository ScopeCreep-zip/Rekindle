use super::*;
use crate::repo::fixture::{self, OWNER};

#[test]
fn hide_and_unhide_are_idempotent_and_listed() {
    let conn = fixture::conn();
    assert!(list(&conn, OWNER).unwrap().is_empty());
    hide(&conn, OWNER, 4181).unwrap();
    hide(&conn, OWNER, 4181).unwrap();
    hide(&conn, OWNER, 12).unwrap();
    assert_eq!(list(&conn, OWNER).unwrap(), vec![12, 4181]);
    assert!(is_hidden(&conn, OWNER, 4181).unwrap());
    unhide(&conn, OWNER, 4181).unwrap();
    unhide(&conn, OWNER, 4181).unwrap();
    assert_eq!(list(&conn, OWNER).unwrap(), vec![12]);
    assert!(!is_hidden(&conn, OWNER, 4181).unwrap());
    assert!(hide(&conn, OWNER, 0).is_err(), "game id 0 is no game");
}
