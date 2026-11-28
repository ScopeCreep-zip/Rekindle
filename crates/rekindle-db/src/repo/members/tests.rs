//! Member rows: owner scoping, rescans, no cached roles, onboarding, timeouts.
use super::*;
use crate::repo::communities::{self, NewCommunity};
use crate::repo::fixture::{self, OWNER};

const OTHER: &str = "other-identity";

fn conn_with_community() -> Connection {
    let conn = fixture::conn();
    conn.execute(
        "INSERT INTO identity (public_key, created_at, friend_request_token) VALUES (?1, 0, 'aaaaaaaaaaaaaaaaaaaaaaaaaa')",
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
                joined_at: 0,
                my_pseudonym_key: "me",
                mek_generation: 0,
                member_registry_key: None,
                my_subkey_index: None,
            },
        )
        .unwrap();
    }
    conn
}

fn discovered(pseudonym_key: &str, subkey: u32) -> DiscoveredMemberRow {
    DiscoveredMemberRow {
        pseudonym_key: pseudonym_key.into(),
        subkey_index: subkey,
        segment_index: 0,
        bio: None,
        pronouns: None,
        theme_color: None,
        badges: Vec::new(),
        avatar_ref: None,
        banner_ref: None,
    }
}

#[test]
fn another_identitys_rows_are_never_read() {
    let conn = conn_with_community();
    upsert_discovered(&conn, OTHER, "c1", &discovered("bob", 3), 1).unwrap();
    set_card(&conn, OTHER, "c1", "bob", (Some("Bob"), None), 1).unwrap();
    assert_eq!(display_name(&conn, OWNER, "c1", "bob").unwrap(), None);
    assert!(display_names(&conn, OWNER, "c1").unwrap().is_empty());
    assert!(roster(&conn, OWNER, "c1").unwrap().is_empty());
    assert_eq!(
        display_name(&conn, OTHER, "c1", "bob").unwrap().as_deref(),
        Some("Bob")
    );
}

#[test]
fn a_rescan_updates_profile_fields_but_keeps_joined_at() {
    let conn = conn_with_community();
    upsert_discovered(&conn, OWNER, "c1", &discovered("bob", 3), 1).unwrap();
    set_card(&conn, OWNER, "c1", "bob", (Some("Robert"), None), 1).unwrap();
    upsert_discovered(&conn, OWNER, "c1", &discovered("bob", 4), 99).unwrap();
    let subkey: u32 = conn
        .query_row(
            "SELECT subkey_index FROM community_members \
             WHERE owner_key = ?1 AND community_id = 'c1' AND pseudonym_key = 'bob'",
            [OWNER],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(subkey, 4);
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
fn no_member_or_community_row_stores_role_ids() {
    // A member's roles, and ours, are the merged governance's role
    // assignments (plan E3.2, D15; review RV7 M1): no row caches them.
    let conn = conn_with_community();
    insert_if_absent(&conn, OWNER, "c1", "bob", 1).unwrap();
    assert_eq!(
        roster(&conn, OWNER, "c1").unwrap(),
        [RosterRow {
            pseudonym_key: "bob".into(),
            display_name: None,
        }]
    );
    for table in ["community_members", "communities"] {
        let columns: Vec<String> = conn
            .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(
            !columns.iter().any(|c| c.contains("role_ids")),
            "{table}: {columns:?}"
        );
    }
}

#[test]
fn onboarding_from_another_device_marks_only_our_own_row() {
    let conn = conn_with_community();
    insert_if_absent(&conn, OWNER, "c1", "me", 1).unwrap();
    insert_if_absent(&conn, OWNER, "c1", "bob", 1).unwrap();
    insert_if_absent(&conn, OTHER, "c1", "me", 1).unwrap();
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
fn a_bootstrap_member_is_stored_once() {
    let conn = conn_with_community();
    let member = MemberInfo {
        pseudonym_key: "bob".into(),
        display_name: "Bob".into(),
        role_ids: vec!["00".repeat(16), "01".repeat(16)],
        status: "online".into(),
        route_blob: None,
        bio: Some("hi".into()),
        pronouns: None,
        theme_color: Some(7),
        badges: vec!["early".into()],
        last_seen: 0,
    };
    insert_bootstrap_if_absent(&conn, OWNER, "c1", &member, 1).unwrap();
    insert_bootstrap_if_absent(&conn, OWNER, "c1", &member, 2).unwrap();
    let profiles = load_profiles(&conn, OWNER).unwrap();
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0].badges, ["early"]);
    assert_eq!(roster(&conn, OWNER, "c1").unwrap().len(), 1);
    delete(&conn, OWNER, "c1", "bob").unwrap();
    assert!(load_profiles(&conn, OWNER).unwrap().is_empty());
}

/// A member's name and custom status are its verified card's alone (plan
/// E3.4 S6): a join row, a registry rescan and a bootstrap list never
/// name it, and a card write keeps the row's place.
#[test]
fn only_the_verified_card_names_a_member() {
    let conn = conn_with_community();
    insert_if_absent(&conn, OWNER, "c1", "bob", 1).unwrap();
    upsert_discovered(&conn, OWNER, "c1", &discovered("bob", 3), 1).unwrap();
    let carol = MemberInfo {
        pseudonym_key: "carol".into(),
        display_name: "Carol says so".into(),
        role_ids: Vec::new(),
        status: "online".into(),
        route_blob: None,
        bio: None,
        pronouns: None,
        theme_color: None,
        badges: Vec::new(),
        last_seen: 0,
    };
    insert_bootstrap_if_absent(&conn, OWNER, "c1", &carol, 1).unwrap();
    assert!(display_names(&conn, OWNER, "c1").unwrap().is_empty());
    set_card(&conn, OWNER, "c1", "bob", (Some("Bob"), Some("afk")), 9).unwrap();
    upsert_discovered(&conn, OWNER, "c1", &discovered("bob", 4), 99).unwrap();
    let bob = load_profiles(&conn, OWNER)
        .unwrap()
        .into_iter()
        .find(|p| p.pseudonym_key == "bob")
        .unwrap();
    assert_eq!(
        (bob.display_name.as_deref(), bob.custom_status.as_deref()),
        (Some("Bob"), Some("afk"))
    );
    set_card(&conn, OWNER, "c1", "bob", (None, None), 10).unwrap();
    assert_eq!(display_name(&conn, OWNER, "c1", "bob").unwrap(), None);
}
