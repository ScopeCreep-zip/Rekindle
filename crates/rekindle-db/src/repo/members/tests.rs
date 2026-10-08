//! Member rows: owner scoping, rescans, roles, onboarding, timeouts.
use super::*;
use crate::repo::communities::{self, NewCommunity};
use crate::repo::fixture::{self, OWNER};

const OTHER: &str = "other-identity";

fn conn_with_community() -> Connection {
    let conn = fixture::conn();
    conn.execute(
        "INSERT INTO identity (public_key, created_at) VALUES (?1, 0)",
        [OTHER],
    )
    .unwrap();
    for owner in [OWNER, OTHER] {
        communities::insert(
            &conn,
            owner,
            &NewCommunity {
                id: "c1",
                name: "One",
                my_role_ids: &[0, 1],
                joined_at: 0,
                dht_owner_keypair: None,
                my_pseudonym_key: "me",
                mek_generation: 0,
                member_registry_key: None,
                my_subkey_index: None,
                governance_key: None,
            },
        )
        .unwrap();
    }
    conn
}

fn discovered(pseudonym_key: &str, name: &str, subkey: i64) -> DiscoveredMemberRow {
    DiscoveredMemberRow {
        pseudonym_key: pseudonym_key.into(),
        display_name: Some(name.into()),
        role_ids_json: "[0,1]".into(),
        subkey_index: subkey,
        segment_index: 0,
        bio: None,
        pronouns: None,
        theme_color: None,
        badges_json: "[]".into(),
        avatar_ref: None,
        banner_ref: None,
    }
}

#[test]
fn another_identitys_rows_are_never_read() {
    let conn = conn_with_community();
    upsert_discovered(&conn, OTHER, "c1", &discovered("bob", "Bob", 3), 1).unwrap();
    assert_eq!(slot(&conn, OWNER, "c1", "bob").unwrap(), None);
    assert_eq!(display_name(&conn, OWNER, "c1", "bob").unwrap(), None);
    assert!(display_names(&conn, OWNER, "c1").unwrap().is_empty());
    assert!(roster(&conn, OWNER, "c1").unwrap().is_empty());
    assert_eq!(slot(&conn, OTHER, "c1", "bob").unwrap(), Some((3, 0)));
}

#[test]
fn a_rescan_updates_profile_fields_but_keeps_joined_at() {
    let conn = conn_with_community();
    upsert_discovered(&conn, OWNER, "c1", &discovered("bob", "Bob", 3), 1).unwrap();
    upsert_discovered(&conn, OWNER, "c1", &discovered("bob", "Robert", 4), 99).unwrap();
    assert_eq!(slots(&conn, OWNER, "c1").unwrap(), [("bob".to_string(), 4)]);
    assert_eq!(
        any_display_name(&conn, OWNER, "bob").unwrap().as_deref(),
        Some("Robert")
    );
    let joined: i64 = conn
        .query_row(
            "SELECT joined_at FROM community_members WHERE pseudonym_key = 'bob'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(joined, 1);
}

#[test]
fn roles_are_added_removed_and_dropped_everywhere() {
    let conn = conn_with_community();
    insert_if_absent(&conn, OWNER, "c1", "bob", "Bob", &[0, 1], 1).unwrap();
    insert_if_absent(&conn, OWNER, "c1", "me", "Me", &[0, 1], 1).unwrap();
    add_role(&conn, OWNER, "c1", "bob", 5).unwrap();
    add_role(&conn, OWNER, "c1", "bob", 5).unwrap();
    add_role(&conn, OWNER, "c1", "ghost", 5).unwrap();
    assert_eq!(
        role_ids(&conn, OWNER, "c1", "bob").unwrap(),
        Some(vec![0, 1, 5])
    );
    assert_eq!(role_ids(&conn, OWNER, "c1", "ghost").unwrap(), None);
    remove_role(&conn, OWNER, "c1", "bob", 0).unwrap();
    assert_eq!(
        role_ids(&conn, OWNER, "c1", "bob").unwrap(),
        Some(vec![1, 5])
    );
    remove_role_everywhere(&conn, OWNER, "c1", 1).unwrap();
    assert_eq!(role_ids(&conn, OWNER, "c1", "bob").unwrap(), Some(vec![5]));
    assert_eq!(role_ids(&conn, OWNER, "c1", "me").unwrap(), Some(vec![0]));
    assert_eq!(
        communities::my_role_ids(&conn, OWNER, "c1").unwrap(),
        Some(vec![0])
    );
    assert_eq!(
        communities::my_role_ids(&conn, OTHER, "c1").unwrap(),
        Some(vec![0, 1])
    );
}

#[test]
fn onboarding_from_another_device_marks_only_our_own_row() {
    let conn = conn_with_community();
    insert_if_absent(&conn, OWNER, "c1", "me", "Me", &[0], 1).unwrap();
    insert_if_absent(&conn, OWNER, "c1", "bob", "Bob", &[0], 1).unwrap();
    insert_if_absent(&conn, OTHER, "c1", "me", "Me", &[0], 1).unwrap();
    set_my_onboarding_complete(&conn, OWNER, "c1").unwrap();
    let done = |owner: &str, pseudonym: &str| -> i64 {
        conn.query_row(
            "SELECT onboarding_complete FROM community_members \
             WHERE owner_key = ?1 AND pseudonym_key = ?2",
            [owner, pseudonym],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(
        (done(OWNER, "me"), done(OWNER, "bob"), done(OTHER, "me")),
        (1, 0, 0)
    );
}

#[test]
fn timeouts_set_and_clear_and_a_bootstrap_member_is_stored_once() {
    let conn = conn_with_community();
    let member = MemberInfo {
        pseudonym_key: "bob".into(),
        display_name: "Bob".into(),
        role_ids: vec![0, 1],
        status: "online".into(),
        timeout_until: None,
        route_blob: None,
        bio: Some("hi".into()),
        pronouns: None,
        theme_color: Some(7),
        badges: vec!["early".into()],
        last_seen: 0,
    };
    insert_bootstrap_if_absent(&conn, OWNER, "c1", &member, 1).unwrap();
    insert_bootstrap_if_absent(&conn, OWNER, "c1", &member, 2).unwrap();
    set_timeout(&conn, OWNER, "c1", "bob", Some(500)).unwrap();
    assert_eq!(
        roster(&conn, OWNER, "c1").unwrap()[0].timeout_until,
        Some(500)
    );
    set_timeout(&conn, OWNER, "c1", "bob", None).unwrap();
    let profiles = load_profiles(&conn, OWNER).unwrap();
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0].badges_json, r#"["early"]"#);
    assert_eq!(roster(&conn, OWNER, "c1").unwrap()[0].timeout_until, None);
    delete(&conn, OWNER, "c1", "bob").unwrap();
    assert!(load_profiles(&conn, OWNER).unwrap().is_empty());
}
