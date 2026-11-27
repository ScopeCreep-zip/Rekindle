use super::*;
use crate::repo::fixture::{self, OWNER};
use crate::repo::group_dms::{self, GroupDmRow};
use rekindle_types::group_dm::GroupDmRosterMember;

fn db() -> Connection {
    let conn = fixture::conn();
    let member = |n: char| GroupDmRosterMember {
        identity: n.to_string().repeat(64),
        profile_record_key: format!("VLD0:{}", "P".repeat(43)),
        display_name: format!("member {n}"),
    };
    let group = GroupDmRow {
        group_id: "ab".repeat(16),
        name: "raid night".to_string(),
        roster: vec![member('a'), member('b'), member('c')],
        epoch: 0,
        created_at: 5,
        origin_peer: None,
        favourite: false,
    };
    group_dms::insert(&conn, OWNER, &group).expect("group");
    conn
}

/// Accept stores the slot; a grow replaces the plan; leaving (the group
/// deleted) deletes the row.
#[test]
fn a_place_is_stored_replaced_and_goes_with_its_group() {
    let conn = db();
    let gid = "ab".repeat(16);
    let mut row = RendezvousRow {
        group_id: gid.clone(),
        record_key: "VLD0:rendezvous".into(),
        slot: 1,
        writer: "VLD0:writer-1".into(),
        slots: vec![
            ("a".repeat(64), 0),
            ("b".repeat(64), 1),
            ("c".repeat(64), 2),
        ],
    };
    put(&conn, OWNER, &row).expect("put");
    assert_eq!(get(&conn, OWNER, &gid).expect("get"), Some(row.clone()));
    assert_eq!(all(&conn, OWNER).expect("all"), vec![row.clone()]);
    row.slots.push(("d".repeat(64), 3));
    put(&conn, OWNER, &row).expect("replace");
    assert_eq!(get(&conn, OWNER, &gid).expect("get"), Some(row.clone()));
    assert!(!format!("{row:?}").contains("writer-1"));
    row.slot = 8;
    assert!(put(&conn, OWNER, &row).is_err());
    conn.execute("DELETE FROM group_dms WHERE group_id = ?1", [&gid])
        .expect("leave");
    assert_eq!(get(&conn, OWNER, &gid).expect("get"), None);
    assert!(all(&conn, OWNER).expect("all").is_empty());
}
