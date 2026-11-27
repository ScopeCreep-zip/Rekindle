use super::*;
use crate::repo::fixture::{self, OWNER};

/// Plan RV2: a friend, our pending request's peer and a requester we
/// hold count as open; a stranger and a peer we are removing do not.
#[test]
fn only_friends_and_open_handshakes_are_open() {
    let conn = fixture::conn();
    insert_accepted(&conn, OWNER, "ada", "Ada", 1).unwrap();
    insert_pending_out(&conn, OWNER, "bo", "Bo", 1).unwrap();
    insert_accepted(&conn, OWNER, "dee", "Dee", 1).unwrap();
    conn.execute(
        "UPDATE friends SET friendship_state = 'removing' WHERE public_key = 'dee'",
        [],
    )
    .unwrap();
    let request = crate::repo::pending_requests::Incoming {
        public_key: "cy",
        display_name: "Cy",
        message: "hi",
        received_at: 1,
        profile_dht_key: "p",
        route_blob: &[],
        mailbox_dht_key: "m",
        invite_id: None,
        reply_inbox_key: "r",
        reply_writer: "w",
        chain_record_key: "VLD0:chain",
    };
    crate::repo::pending_requests::upsert(&conn, OWNER, &request).unwrap();
    for (peer, open) in [
        ("ada", true),
        ("bo", true),
        ("cy", true),
        ("dee", false),
        ("zed", false),
    ] {
        assert_eq!(
            open_handshake_or_friend(&conn, OWNER, peer).unwrap(),
            open,
            "{peer}"
        );
    }
}

#[test]
fn an_outgoing_request_loads_as_pending_out_and_accept_does_not_overwrite_it() {
    let conn = fixture::conn();
    insert_pending_out(&conn, OWNER, "bob", "Bob", 1).unwrap();
    insert_accepted(&conn, OWNER, "bob", "Other", 2).unwrap();
    let rows = load_all(&conn, OWNER).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].display_name, "Bob");
    assert_eq!(rows[0].friendship_state, FriendshipState::PendingOut);
}

#[test]
fn set_writes_one_column_and_record_keys_keep_the_stored_value_for_none() {
    let conn = fixture::conn();
    replace_with_invited(&conn, OWNER, "bob", "Bob", 1, "profile", "mailbox").unwrap();
    set_nickname(&conn, OWNER, "bob", Some("bobby")).unwrap();
    set_record_keys(&conn, OWNER, "bob", None, Some("mailbox-2")).unwrap();
    let row = &load_all(&conn, OWNER).unwrap()[0];
    assert_eq!(row.nickname.as_deref(), Some("bobby"));
    assert_eq!(row.dht_record_key.as_deref(), Some("profile"));
    assert_eq!(row.mailbox_dht_key.as_deref(), Some("mailbox-2"));
    let record = record_by_profile_key(&conn, OWNER, "profile")
        .unwrap()
        .unwrap();
    assert_eq!(record.status, FriendshipState::PendingOut);
}

#[test]
fn a_removed_friend_is_forgotten_only_once_its_unfriended_row_settles() {
    use crate::repo::outbox;
    let conn = fixture::conn();
    insert_accepted(&conn, OWNER, "bob", "Bob", 1).unwrap();
    insert_accepted(&conn, OWNER, "bobby", "Bobby", 1).unwrap();
    set_state(&conn, OWNER, "bob", FriendshipState::Removing).unwrap();
    outbox::queue_for_test(
        &conn,
        OWNER,
        "friend_handshake:bob",
        "unfriended:bob",
        b"{}",
    );
    // A send to another peer whose key extends bob's holds nothing.
    outbox::queue_for_test(&conn, OWNER, "dm:bobby", "m1", b"{}");
    assert!(forget_removed(&conn, OWNER).unwrap().is_empty());
    assert_eq!(
        load_all(&conn, OWNER).unwrap()[0].friendship_state,
        FriendshipState::Removing
    );
    // Failed at its deadline: settled for the removal, and forgotten
    // with the friend.
    let failed = outbox::Update {
        state: rekindle_types::message::SendState::Failed,
        attempts: 3,
        next_at: 0,
        deadline: 0,
        wedge_reason: Some("deadline"),
    };
    outbox::update(&conn, OWNER, "unfriended:bob", &failed).unwrap();
    assert_eq!(forget_removed(&conn, OWNER).unwrap(), ["bob"]);
    assert_eq!(load_all(&conn, OWNER).unwrap().len(), 1);
    assert_eq!(
        outbox::unsettled_count(&conn, OWNER).unwrap(),
        1,
        "bobby's stays"
    );
}

#[test]
fn adding_a_friend_we_are_removing_ends_the_removal() {
    let conn = fixture::conn();
    insert_accepted(&conn, OWNER, "bob", "Bob", 1).unwrap();
    set_state(&conn, OWNER, "bob", FriendshipState::Removing).unwrap();
    crate::repo::outbox::queue_for_test(
        &conn,
        OWNER,
        "friend_handshake:bob",
        "unfriended:bob",
        b"{}",
    );
    conn.execute(
        "INSERT INTO ratchet_sessions (owner_key, peer_identity, peer_device, record, updated_at) \
             VALUES (?1, 'bob', 'd', x'00', 1)",
        params![OWNER.to_ascii_lowercase()],
    )
    .unwrap();
    insert_pending_out(&conn, OWNER, "bob", "Bob", 2).unwrap();
    assert_eq!(
        load_all(&conn, OWNER).unwrap()[0].friendship_state,
        FriendshipState::PendingOut
    );
    assert_eq!(
        crate::repo::outbox::unsettled_count(&conn, OWNER).unwrap(),
        0
    );
    let sessions: i64 = conn
        .query_row("SELECT COUNT(*) FROM ratchet_sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(sessions, 1, "the removal's session stays (plan E2.5h)");
}

#[test]
fn cancelling_a_request_keeps_a_session_from_an_ended_friendship() {
    let conn = fixture::conn();
    insert_pending_out(&conn, OWNER, "bob", "Bob", 1).unwrap();
    conn.execute(
        "INSERT INTO ratchet_sessions (owner_key, peer_identity, peer_device, record, updated_at) \
             VALUES (?1, 'bob', 'd', x'00', 1)",
        params![OWNER.to_ascii_lowercase()],
    )
    .unwrap();
    cancel_request(&conn, OWNER, "bob").unwrap();
    assert!(load_all(&conn, OWNER).unwrap().is_empty());
    let sessions: i64 = conn
        .query_row("SELECT COUNT(*) FROM ratchet_sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(sessions, 1);
}

#[test]
fn a_new_request_keeps_a_session_from_an_ended_friendship() {
    let conn = fixture::conn();
    conn.execute(
        "INSERT INTO ratchet_sessions (owner_key, peer_identity, peer_device, record, updated_at) \
             VALUES (?1, 'bob', 'd', x'00', 1)",
        params![OWNER.to_ascii_lowercase()],
    )
    .unwrap();
    insert_pending_out(&conn, OWNER, "bob", "Bob", 1).unwrap();
    let sessions: i64 = conn
        .query_row("SELECT COUNT(*) FROM ratchet_sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(sessions, 1);
}

#[test]
fn a_corrupt_friendship_state_is_a_read_error() {
    let conn = fixture::conn();
    insert_accepted(&conn, OWNER, "bob", "Bob", 1).unwrap();
    conn.pragma_update(None, "ignore_check_constraints", true)
        .unwrap();
    conn.execute("UPDATE friends SET friendship_state = 'active'", [])
        .unwrap();
    let err = load_all(&conn, OWNER).unwrap_err().to_string();
    assert!(err.contains("friendship_state"), "{err}");
    assert!(record(&conn, OWNER, "bob").is_err());
}

#[test]
fn only_old_outgoing_requests_expire() {
    let conn = fixture::conn();
    insert_pending_out(&conn, OWNER, "old", "Old", 1).unwrap();
    insert_pending_out(&conn, OWNER, "new", "New", 100).unwrap();
    insert_accepted(&conn, OWNER, "friend", "Friend", 1).unwrap();
    assert_eq!(
        delete_pending_out_before(&conn, OWNER, 50)
            .unwrap()
            .into_iter()
            .map(|(key, _)| key)
            .collect::<Vec<_>>(),
        ["old"]
    );
    let mut left: Vec<String> = load_all(&conn, OWNER)
        .unwrap()
        .into_iter()
        .map(|r| r.public_key)
        .collect();
    left.sort();
    assert_eq!(left, ["friend", "new"]);
}

#[test]
fn a_friend_without_an_avatar_has_none() {
    let conn = fixture::conn();
    insert_accepted(&conn, OWNER, "bob", "Bob", 1).unwrap();
    assert_eq!(avatar(&conn, OWNER, "bob").unwrap(), None);
    assert_eq!(avatar(&conn, OWNER, "ghost").unwrap(), None);
}
