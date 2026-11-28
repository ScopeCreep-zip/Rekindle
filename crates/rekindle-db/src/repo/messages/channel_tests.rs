use super::*;
use crate::repo::fixture::{self, OWNER};

fn row(id: &str, ts: i64, lamport: u64) -> ChannelMessageInsert<'_> {
    ChannelMessageInsert {
        owner_key: OWNER,
        channel_id: "ch",
        sender_key: "alice",
        body: id,
        timestamp: ts,
        is_read: false,
        mek_generation: Some(1),
        message_id: id,
        lamport_ts: lamport,
        automod_blurred: false,
        forwarded_from_author: None,
        flags: 0,
        attachment_json: None,
        send_state: SendState::Delivered,
        mentions_me: false,
        reply_to: None,
    }
}

#[test]
fn page_is_oldest_first_and_before_bounds_it() {
    let conn = fixture::conn();
    fixture::community(&conn, "c");
    for (id, ts, l) in [("a", 10, 1), ("b", 20, 2), ("c", 30, 3)] {
        insert(&conn, &row(id, ts, l)).unwrap();
    }
    let all: Vec<_> = page(&conn, OWNER, "ch", None, 10).unwrap();
    let ids: Vec<_> = all.iter().map(|m| m.body.as_str()).collect();
    assert_eq!(ids, ["a", "b", "c"]);
    let older = page(&conn, OWNER, "ch", Some(30), 10).unwrap();
    assert_eq!(older.len(), 2);
    assert_eq!(
        find_by_message_id(&conn, OWNER, "ch", "b")
            .unwrap()
            .unwrap()
            .body,
        "b"
    );
}

#[test]
fn a_failed_posts_reason_is_kept_for_the_reload() {
    let conn = fixture::conn();
    fixture::community(&conn, "c");
    let sent = ChannelMessageInsert {
        sender_key: OWNER,
        send_state: SendState::Sent,
        ..row("ours", 10, 1)
    };
    insert(&conn, &sent).unwrap();
    insert(&conn, &row("theirs", 20, 2)).unwrap();
    let reason = "Not delivered: you were removed from the community";
    assert!(fail_own_post(&conn, OWNER, "ours", reason).unwrap());
    // A received message has no send to fail.
    assert!(!fail_own_post(&conn, OWNER, "theirs", reason).unwrap());
    let read = page(&conn, OWNER, "ch", None, 10).unwrap();
    assert_eq!(read[0].send_state, SendState::Failed);
    assert_eq!(read[0].failure_reason.as_deref(), Some(reason));
    assert_eq!(read[1].send_state, SendState::Delivered);
    assert_eq!(read[1].failure_reason, None);
}

#[test]
fn the_notification_gate_reads_the_stored_mention_and_flags() {
    let conn = fixture::conn();
    fixture::community(&conn, "c");
    let mention = ChannelMessageInsert {
        mentions_me: true,
        flags: 4,
        ..row("ping", 10, 1)
    };
    insert(&conn, &mention).unwrap();
    insert(&conn, &row("plain", 20, 2)).unwrap();
    assert_eq!(
        notify_fields(&conn, OWNER, "ping").unwrap(),
        Some((true, 4))
    );
    assert_eq!(
        notify_fields(&conn, OWNER, "plain").unwrap(),
        Some((false, 0))
    );
    assert_eq!(notify_fields(&conn, OWNER, "absent").unwrap(), None);
}

#[test]
fn history_ranges_span_each_channels_stamped_messages() {
    let conn = fixture::conn();
    fixture::community(&conn, "c");
    conn.execute(
        "INSERT INTO channels (owner_key, id, community_id, name, channel_type) \
         VALUES (?1, 'ch', 'c', 'general', 'text')",
        params![OWNER],
    )
    .unwrap();
    for (id, ts, l) in [("a", 10, 4), ("b", 20, 9), ("c", 30, 0)] {
        insert(&conn, &row(id, ts, l)).unwrap();
    }
    assert_eq!(
        history_ranges(&conn, OWNER, "c").unwrap(),
        vec![("ch".to_string(), 4, 9)]
    );
    assert!(history_ranges(&conn, OWNER, "other").unwrap().is_empty());
}
