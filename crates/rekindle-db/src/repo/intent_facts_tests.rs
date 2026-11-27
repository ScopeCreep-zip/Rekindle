use super::*;
use crate::repo::fixture::{self, OWNER};

#[test]
fn counts_are_per_owner_and_per_container() {
    let conn = fixture::conn();
    fixture::community(&conn, "c");
    for (i, ch) in ["ch1", "ch1", "ch2"].iter().enumerate() {
        conn.execute(
            "INSERT INTO messages (owner_key, conversation_id, conversation_type, sender_key, body, timestamp) \
                 VALUES (?1, ?2, 'channel', 's', 'b', ?3)",
            params![OWNER, ch, i64::try_from(i).unwrap()],
        )
        .unwrap();
    }
    assert_eq!(channel_messages(&conn, OWNER, "ch1").unwrap(), 2);
    assert_eq!(channel_messages(&conn, "other", "ch1").unwrap(), 0);

    for pk in ["a", "b"] {
        conn.execute(
            "INSERT INTO community_members (owner_key, community_id, pseudonym_key, joined_at) \
                 VALUES (?1, 'c', ?2, 0)",
            params![OWNER, pk],
        )
        .unwrap();
    }
    assert_eq!(community_members(&conn, OWNER, "c").unwrap(), 2);

    assert_eq!(unsent(&conn, OWNER).unwrap(), 0);
    crate::repo::outbox::queue_for_test(&conn, OWNER, "dm:r", "m1", b"{}");
    crate::repo::outbox::enqueue_with_echo(
        &conn,
        &crate::repo::outbox::NewRow {
            owner_key: OWNER,
            stream: "dht_write:k/0",
            client_id: "k/0@1",
            kind: "dht_write",
            payload: &[1],
            next_at: 0,
            deadline: 1,
            depends_on: None,
        },
        None,
    )
    .unwrap();
    assert_eq!(unsent(&conn, OWNER).unwrap(), 2);
}
