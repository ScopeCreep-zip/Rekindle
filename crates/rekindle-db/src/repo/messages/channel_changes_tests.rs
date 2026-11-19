use super::*;
use crate::repo::fixture::{self, OWNER};
use crate::repo::messages::channel::{self, ChannelMessageInsert, HeldMessage};
use rekindle_types::message::SendState;

fn seed(conn: &Connection) {
    fixture::community(conn, "c");
    channel::insert(
        conn,
        &ChannelMessageInsert {
            owner_key: OWNER,
            channel_id: "ch",
            sender_key: "alice",
            body: "first",
            timestamp: 10,
            is_read: false,
            mek_generation: Some(1),
            message_id: "m1",
            lamport_ts: 3,
            automod_blurred: false,
            forwarded_from_author: None,
            flags: 0,
            attachment_json: None,
            send_state: SendState::Delivered,
            reply_to: None,
            mentions_me: false,
        },
    )
    .unwrap();
}

fn edit<'a>(author: &'a str, body: &'a str, seq: u64) -> StoredEdit<'a> {
    StoredEdit {
        owner_key: OWNER,
        channel_id: "ch",
        message_id: "m1",
        author,
        body,
        edited_at: 100 + seq,
        seq,
        blurred: false,
    }
}

fn shown(conn: &Connection) -> Vec<(String, Option<i64>)> {
    (channel::page(conn, OWNER, "ch", None, 10)
        .unwrap()
        .into_iter())
    .map(|m| (m.body, m.edited_at))
    .collect()
}

#[test]
fn an_edit_is_stored_and_read_back_on_the_reload() {
    let conn = fixture::conn();
    seed(&conn);
    assert!(store_edit(&conn, &edit("alice", "second", 5)).unwrap());
    assert_eq!(shown(&conn), [("second".to_owned(), Some(105))]);
    let target = change_target(&conn, OWNER, "ch", "m1").unwrap().unwrap();
    assert_eq!((target.edit_seq, target.edit_count), (5, 1));
}

#[test]
fn only_the_author_edits_and_the_latest_seq_wins() {
    let conn = fixture::conn();
    seed(&conn);
    assert!(
        !store_edit(&conn, &edit("mallory", "forged", 9)).unwrap(),
        "E1: another author's edit is refused"
    );
    assert!(store_edit(&conn, &edit("alice", "latest", 8)).unwrap());
    assert!(
        !store_edit(&conn, &edit("alice", "older", 6)).unwrap(),
        "E1: an edit below the shown one's seq is refused"
    );
    assert!(!store_edit(&conn, &edit("alice", "same", 8)).unwrap());
    assert_eq!(shown(&conn), [("latest".to_owned(), Some(108))]);
}

#[test]
fn a_reader_caps_a_message_at_ten_edits() {
    let conn = fixture::conn();
    seed(&conn);
    for seq in 1..=u64::from(MAX_EDITS) {
        assert!(store_edit(&conn, &edit("alice", "again", 10 + seq)).unwrap());
    }
    assert!(
        !store_edit(&conn, &edit("alice", "eleventh", 99)).unwrap(),
        "E2: the eleventh edit is refused"
    );
    assert_eq!(shown(&conn)[0].0, "again");
}

#[test]
fn a_tombstone_is_final_and_purges_the_revisions() {
    let conn = fixture::conn();
    seed(&conn);
    assert!(store_edit(&conn, &edit("alice", "second", 5)).unwrap());
    assert!(
        !tombstone(&conn, OWNER, "ch", "m1", "mallory", 200).unwrap(),
        "only the author deletes its message"
    );
    assert!(tombstone(&conn, OWNER, "ch", "m1", "alice", 200).unwrap());
    assert!(
        !store_edit(&conn, &edit("alice", "after", 9)).unwrap(),
        "E3: an edit after the tombstone is refused"
    );
    assert!(shown(&conn).is_empty(), "a deleted message is not shown");
    assert_eq!(
        channel::held(&conn, OWNER, "ch", "m1").unwrap(),
        Some(HeldMessage::Deleted),
        "the row stays, so the author's record never restores it"
    );
    let body: String = conn
        .query_row(
            "SELECT body FROM messages WHERE message_id = 'm1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(body.is_empty(), "every revision is purged");
    assert!(!tombstone(&conn, OWNER, "ch", "m1", "alice", 300).unwrap());
}

fn removal(by: &str) -> AdminTombstone<'_> {
    AdminTombstone {
        owner_key: OWNER,
        channel_id: "ch",
        message_id: "m1",
        by,
        author: "alice",
        deleted_at: 300,
    }
}

#[test]
fn a_moderators_delete_is_a_tombstone_shown_as_removed_by() {
    let conn = fixture::conn();
    seed(&conn);
    assert!(store_edit(&conn, &edit("alice", "second", 5)).unwrap());
    assert!(
        admin_tombstone(&conn, &removal("mod")).unwrap(),
        "any author's message"
    );
    assert!(
        !store_edit(&conn, &edit("alice", "after", 9)).unwrap(),
        "delete wins: a later edit is refused"
    );
    assert!(
        !admin_tombstone(&conn, &removal("other")).unwrap(),
        "the first moderator stays"
    );
    let page = channel::page(&conn, OWNER, "ch", None, 10).unwrap();
    assert_eq!(page.len(), 1, "the removed message stays in the page");
    assert_eq!(page[0].body, "", "every revision is purged");
    assert_eq!(page[0].removed_by.as_deref(), Some("mod"));
    assert_eq!(page[0].edited_at, None);
}

#[test]
fn a_moderators_delete_after_the_authors_changes_nothing() {
    let conn = fixture::conn();
    seed(&conn);
    assert!(tombstone(&conn, OWNER, "ch", "m1", "alice", 200).unwrap());
    assert!(!admin_tombstone(&conn, &removal("mod")).unwrap());
    assert!(channel::page(&conn, OWNER, "ch", None, 10)
        .unwrap()
        .is_empty());
}

#[test]
fn a_moderators_delete_naming_another_author_changes_nothing() {
    let conn = fixture::conn();
    seed(&conn);
    let delete = AdminTombstone {
        author: "mallory",
        ..removal("mod")
    };
    assert!(
        !admin_tombstone(&conn, &delete).unwrap(),
        "M1 judged the rank against the author the delete names"
    );
    assert_eq!(shown(&conn)[0].0, "first");
}
