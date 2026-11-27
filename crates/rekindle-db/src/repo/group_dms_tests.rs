use super::*;
use crate::repo::fixture::{self, OWNER};

const G: &str = "0123456789abcdef0123456789abcdef";

fn member(n: char) -> GroupDmRosterMember {
    GroupDmRosterMember {
        identity: n.to_string().repeat(64),
        profile_record_key: format!("VLD0:{}", "P".repeat(43)),
        display_name: format!("member {n}"),
    }
}

fn group() -> GroupDmRow {
    GroupDmRow {
        group_id: G.to_string(),
        name: "raid night".to_string(),
        roster: vec![member('a'), member('b'), member('c')],
        epoch: 0,
        created_at: 5,
        origin_peer: None,
        favourite: false,
    }
}

#[test]
fn a_group_is_stored_listed_updated_and_deleted() {
    let conn = fixture::conn();
    insert(&conn, OWNER, &group()).unwrap();
    assert!(insert(&conn, OWNER, &group()).is_err(), "held once");
    assert_eq!(get(&conn, OWNER, G).unwrap(), Some(group()));
    assert_eq!(list(&conn, OWNER).unwrap(), vec![group()]);
    let roster = vec![member('a'), member('b')];
    assert!(update_roster_epoch(&conn, OWNER, G, &roster, 1).unwrap());
    let held = get(&conn, OWNER, G).unwrap().unwrap();
    assert_eq!((held.roster, held.epoch), (roster, 1));
    assert!(!update_roster_epoch(&conn, OWNER, "ff", &[], 1).unwrap());
    assert!(delete(&conn, OWNER, G).unwrap());
    assert_eq!(get(&conn, OWNER, G).unwrap(), None);
    assert!(!delete(&conn, OWNER, G).unwrap());
}

#[test]
fn a_rename_changes_the_name_once_and_only_a_held_group() {
    let conn = fixture::conn();
    insert(&conn, OWNER, &group()).unwrap();
    assert!(set_name(&conn, OWNER, G, "raid night II").unwrap());
    assert_eq!(get(&conn, OWNER, G).unwrap().unwrap().name, "raid night II");
    assert!(
        !set_name(&conn, OWNER, G, "raid night II").unwrap(),
        "unchanged"
    );
    assert!(!set_name(&conn, OWNER, "ff", "x").unwrap(), "not held");
}

#[test]
fn a_corrupt_roster_or_id_is_refused() {
    let conn = fixture::conn();
    let mut bad = group();
    bad.group_id = "short".to_string();
    assert!(insert(&conn, OWNER, &bad).is_err());
    insert(&conn, OWNER, &group()).unwrap();
    conn.execute("UPDATE group_dms SET roster = '[1]'", [])
        .unwrap();
    assert!(get(&conn, OWNER, G).is_err(), "a roster entry is a member");
}

#[test]
fn chains_are_kept_by_member_and_go_with_the_group() {
    let conn = fixture::conn();
    let (a, b) = ("a".repeat(64), "b".repeat(64));
    assert!(
        put_chain(&conn, OWNER, G, &a, "VLD0:x").is_err(),
        "no such group"
    );
    insert(&conn, OWNER, &group()).unwrap();
    put_chain(&conn, OWNER, G, &a, "VLD0:x").unwrap();
    put_chain(&conn, OWNER, G, &a, "VLD0:y").unwrap();
    put_chain(&conn, OWNER, G, &b, "VLD0:z").unwrap();
    assert_eq!(
        chains(&conn, OWNER, G).unwrap(),
        vec![(a.clone(), "VLD0:y".into()), (b.clone(), "VLD0:z".into())]
    );
    assert_eq!(delete_chain(&conn, OWNER, G, &a).unwrap(), 1);
    delete(&conn, OWNER, G).unwrap();
    assert!(chains(&conn, OWNER, G).unwrap().is_empty(), "cascade");
}

#[test]
fn sender_keys_are_kept_by_member_and_epoch() {
    let conn = fixture::conn();
    let (a, b) = ("a".repeat(64), "b".repeat(64));
    assert!(
        put_sender_key(&conn, OWNER, G, &a, 0, &[1; 32]).is_err(),
        "no such group"
    );
    insert(&conn, OWNER, &group()).unwrap();
    put_sender_key(&conn, OWNER, G, &a, 0, &[1; 32]).unwrap();
    put_sender_key(&conn, OWNER, G, &a, 1, &[2; 32]).unwrap();
    put_sender_key(&conn, OWNER, G, &b, 0, &[3; 32]).unwrap();
    assert_eq!(sender_key(&conn, OWNER, G, &a, 0).unwrap(), Some([1; 32]));
    assert_eq!(sender_key(&conn, OWNER, G, &a, 2).unwrap(), None);
    assert_eq!(
        latest_sender_key(&conn, OWNER, G, &a).unwrap(),
        Some((1, [2; 32]))
    );
    assert_eq!(delete_member_keys(&conn, OWNER, G, &a).unwrap(), 2);
    assert_eq!(latest_sender_key(&conn, OWNER, G, &a).unwrap(), None);
    assert!(conn
        .execute(
            "INSERT INTO group_dm_sender_keys VALUES (?1, ?2, ?3, 5, x'00')",
            params![OWNER, G, b],
        )
        .is_err());
    delete(&conn, OWNER, G).unwrap();
    assert_eq!(
        latest_sender_key(&conn, OWNER, G, &b).unwrap(),
        None,
        "cascade"
    );
}

#[test]
fn the_attached_room_is_the_latest_created_from_that_chat() {
    let conn = fixture::conn();
    let peer = "b".repeat(64);
    assert_eq!(attached(&conn, OWNER, &peer).unwrap(), None);
    let mut older = group();
    older.created_at = 1;
    insert(&conn, OWNER, &older).unwrap();
    assert!(set_origin_peer(&conn, OWNER, G, &peer).unwrap());
    let mut newer = group();
    newer.group_id = "f".repeat(32);
    newer.created_at = 2;
    newer.origin_peer = Some(peer.clone());
    insert(&conn, OWNER, &newer).unwrap();
    assert_eq!(attached(&conn, OWNER, &peer).unwrap(), Some("f".repeat(32)));
    assert_eq!(
        get(&conn, OWNER, G).unwrap().unwrap().origin_peer,
        Some(peer)
    );
    assert!(!set_origin_peer(&conn, OWNER, &"e".repeat(32), &"c".repeat(64)).unwrap());
}
