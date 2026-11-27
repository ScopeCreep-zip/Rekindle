use rekindle_types::search::{MessageSearch, SearchFilters, SearchSort};

use super::{get, insert_message, message_rows, upsert, Connection, ThreadInfo, ThreadMessageRow};
use crate::repo::fixture::{self, OWNER};
use crate::repo::search::search;

const COMMUNITY: &str = "community";
const THREAD: &str = "thread";

fn with_thread() -> Connection {
    let conn = fixture::conn();
    fixture::community(&conn, COMMUNITY);
    let thread = ThreadInfo {
        id: THREAD.into(),
        channel_id: "channel".into(),
        name: "thread".into(),
        starter_message_id: "starter".into(),
        creator_pseudonym: "bb22".into(),
        forum_tag: None,
        created_at: 1,
        archived: false,
        auto_archive_seconds: 3600,
        last_message_at: 1,
        message_count: 0,
    };
    upsert(&conn, OWNER, COMMUNITY, &thread).unwrap();
    conn
}

#[test]
fn thread_reply_id_is_scoped_to_its_verified_author() {
    let conn = with_thread();
    let mut reply = ThreadMessageRow {
        thread_id: THREAD,
        message_id: "tmsg_shared",
        sender_pseudonym: "aa11",
        body: "first reply",
        timestamp: 2,
        reply_to_id: None,
    };
    insert_message(&conn, OWNER, COMMUNITY, &reply).unwrap();
    reply.sender_pseudonym = "bb22";
    reply.body = "genuine reply";
    reply.timestamp = 3;
    insert_message(&conn, OWNER, COMMUNITY, &reply).unwrap();

    assert_eq!(message_rows(&conn, OWNER, COMMUNITY, THREAD).unwrap(), 2);
    let hits = search(
        &conn,
        OWNER,
        &MessageSearch {
            query: "reply".into(),
            filters: SearchFilters::default(),
            sort: SearchSort::Oldest,
            limit: 10,
            offset: 0,
        },
    )
    .unwrap();
    let replies: Vec<_> = hits
        .iter()
        .map(|hit| (&*hit.sender_key, hit.message_id.as_deref(), &*hit.body))
        .collect();
    assert_eq!(
        replies,
        [
            ("aa11", Some("aa11:tmsg_shared"), "first reply"),
            ("bb22", Some("bb22:tmsg_shared"), "genuine reply"),
        ],
    );
}

#[test]
fn repeated_thread_reply_notification_stores_and_counts_one_row() {
    let conn = with_thread();
    let reply = ThreadMessageRow {
        thread_id: THREAD,
        message_id: "tmsg_repeat",
        sender_pseudonym: "aa11",
        body: "reply",
        timestamp: 2,
        reply_to_id: None,
    };
    for _ in 0..2 {
        insert_message(&conn, OWNER, COMMUNITY, &reply).unwrap();
    }
    assert_eq!(message_rows(&conn, OWNER, COMMUNITY, THREAD).unwrap(), 1);
    let thread = get(&conn, OWNER, COMMUNITY, THREAD).unwrap().unwrap();
    assert_eq!(thread.message_count, 1);
}
