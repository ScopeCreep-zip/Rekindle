use super::*;
use crate::repo::fixture::{self, OWNER};

#[test]
fn a_deleted_group_leaves_its_friends_ungrouped() {
    let conn = fixture::conn();
    let id = create(&conn, OWNER, "Raid").unwrap();
    assert!(rename(&conn, OWNER, id, "Clan").unwrap());
    assert_eq!(name(&conn, OWNER, id).unwrap().as_deref(), Some("Clan"));
    crate::repo::friends::insert_accepted(&conn, OWNER, "bob", "Bob", 1).unwrap();
    crate::repo::friends::set(
        &conn,
        OWNER,
        "bob",
        crate::repo::friends::Column::GroupId,
        id,
    )
    .unwrap();
    let row = &crate::repo::friends::load_all(&conn, OWNER).unwrap()[0];
    assert_eq!(row.group_id, Some(id));
    assert_eq!(row.group_name.as_deref(), Some("Clan"));
    assert!(delete(&conn, OWNER, id).unwrap());
    assert!(!rename(&conn, OWNER, id, "Gone").unwrap());
    let row = &crate::repo::friends::load_all(&conn, OWNER).unwrap()[0];
    assert_eq!(row.group_name, None);
    assert_eq!(row.group_id, None);
}
