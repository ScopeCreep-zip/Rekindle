use super::*;
use crate::repo::fixture::{conn, OWNER};
use crate::repo::{blocked, friends, outbox};

const BOB: &str = "bb";

fn nickname_of(conn: &Connection, key: &str) -> Option<String> {
    friends::names(conn, OWNER, key).unwrap().and_then(|n| n.0)
}

fn named_friend(conn: &Connection) {
    friends::insert_accepted(conn, OWNER, BOB, "Bob", 1).unwrap();
    set_nickname(conn, OWNER, BOB, Some("Mac buddy")).unwrap();
    assert_eq!(nickname_of(conn, BOB).as_deref(), Some("Mac buddy"));
}

#[test]
fn a_removal_clears_the_nickname_and_a_re_add_starts_clean() {
    let conn = conn();
    named_friend(&conn);
    let stream = format!("friend_handshake:{BOB}");
    let row = outbox::NewRow {
        owner_key: OWNER,
        stream: &stream,
        client_id: "unfriended:bb",
        kind: "friend_handshake",
        payload: b"{}",
        next_at: 2,
        deadline: 9,
        depends_on: None,
    };
    friends::mark_removing(&conn, OWNER, BOB, &row).unwrap();
    assert_eq!(nickname_of(&conn, BOB), None, "gone while removing");
    friends::insert_pending_out(&conn, OWNER, BOB, "Bob", 3).unwrap();
    assert_eq!(nickname_of(&conn, BOB), None, "the re-add starts clean");
}

#[test]
fn being_unfriended_or_cancelling_clears_it() {
    let conn = conn();
    named_friend(&conn);
    friends::unfriended(&conn, OWNER, BOB).unwrap();
    friends::insert_accepted(&conn, OWNER, BOB, "Bob", 2).unwrap();
    assert_eq!(nickname_of(&conn, BOB), None);
    set_nickname(&conn, OWNER, BOB, Some("Mac buddy")).unwrap();
    friends::cancel_request(&conn, OWNER, BOB).unwrap();
    friends::insert_accepted(&conn, OWNER, BOB, "Bob", 3).unwrap();
    assert_eq!(nickname_of(&conn, BOB), None);
}

#[test]
fn a_block_and_unblock_of_the_same_key_keep_it() {
    let conn = conn();
    named_friend(&conn);
    friends::block(&conn, OWNER, BOB, "Bob", 2).unwrap();
    blocked::delete(&conn, OWNER, BOB).unwrap();
    friends::insert_accepted(&conn, OWNER, BOB, "Bob", 3).unwrap();
    assert_eq!(nickname_of(&conn, BOB).as_deref(), Some("Mac buddy"));
}

#[test]
fn after_a_key_change_it_names_the_previous_key_until_set_again() {
    let conn = conn();
    named_friend(&conn);
    conn.execute(
        "UPDATE friends SET public_key = 'cc' WHERE public_key = ?1",
        params![BOB],
    )
    .unwrap();
    rekey(&conn, OWNER, BOB, "cc").unwrap();
    assert_eq!(
        nickname_of(&conn, "cc"),
        Some(for_previous_key("Mac buddy"))
    );
    assert_eq!(
        friends::load_all(&conn, OWNER).unwrap()[0].nickname,
        Some(for_previous_key("Mac buddy"))
    );
    set_nickname(&conn, OWNER, "cc", Some("Mac buddy")).unwrap();
    assert_eq!(nickname_of(&conn, "cc").as_deref(), Some("Mac buddy"));
}
