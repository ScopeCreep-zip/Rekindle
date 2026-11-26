use super::*;
use crate::repo::fixture::{self, OWNER};

const ROW: PartialRow = PartialRow {
    height: 2,
    digest: [7; 32],
    state_root: [8; 32],
};

#[test]
fn a_partial_community_reads_back_until_cleared() {
    let conn = fixture::conn();
    fixture::community(&conn, "c1");
    fixture::community(&conn, "c2");
    assert_eq!(get(&conn, OWNER, "c1").unwrap(), None);
    set(&conn, OWNER, "c1", &ROW).unwrap();
    assert_eq!(get(&conn, OWNER, "c1").unwrap(), Some(ROW));
    assert_eq!(get(&conn, OWNER, "c2").unwrap(), None);
    clear(&conn, OWNER, "c1").unwrap();
    assert_eq!(get(&conn, OWNER, "c1").unwrap(), None);
}

#[test]
fn governance_partial_refuses_an_unknown_community() {
    let conn = fixture::conn();
    assert!(set(&conn, OWNER, "gone", &ROW).is_err());
}
