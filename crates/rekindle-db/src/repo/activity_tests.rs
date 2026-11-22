use rusqlite::params;

use super::*;
use crate::repo::fixture::{self, OWNER};

const FEED_NOW: i64 = 100 * MAX_AGE_MS;

fn friend(conn: &Connection, key: &str) {
    conn.execute(
        "INSERT INTO friends (owner_key, public_key, added_at, friendship_state) \
         VALUES (?1, ?2, 0, 'accepted')",
        params![OWNER, key],
    )
    .unwrap();
}

fn bio(friend: &str, at: i64) -> ActivityEntry {
    ActivityEntry {
        at,
        kind: ActivityKind::BioChanged {
            friend: friend.into(),
        },
    }
}

fn joined(at: i64) -> ActivityEntry {
    ActivityEntry {
        at,
        kind: ActivityKind::JoinedCommunity {
            community: "c1".into(),
        },
    }
}

fn entries(conn: &Connection) -> Vec<ActivityEntry> {
    page(conn, OWNER, None, u16::MAX)
        .unwrap()
        .into_iter()
        .map(|s| s.entry)
        .collect()
}

/// Every kind round-trips, the feed reads newest first (ties by insertion),
/// and `before` pages on from a row; a cursor no longer stored answers
/// nothing.
#[test]
fn entries_round_trip_newest_first_and_page_by_cursor() {
    let mut conn = fixture::conn();
    friend(&conn, "bob");
    let all = vec![
        ActivityEntry {
            at: FEED_NOW - 50,
            kind: ActivityKind::StartedGame {
                friend: "bob".into(),
                game_id: 4181,
                server: Some("203.0.113.7:27015".into()),
            },
        },
        ActivityEntry {
            at: FEED_NOW - 40,
            kind: ActivityKind::StoppedGame {
                friend: "bob".into(),
                game_id: 4181,
                seconds: 90,
            },
        },
        ActivityEntry {
            at: FEED_NOW - 40,
            kind: ActivityKind::FirstPlayed {
                friend: "bob".into(),
                game_id: 12,
            },
        },
        ActivityEntry {
            at: FEED_NOW - 30,
            kind: ActivityKind::RenamedSelf {
                friend: "bob".into(),
                from: "b".into(),
                to: "Bob".into(),
            },
        },
        ActivityEntry {
            at: FEED_NOW - 20,
            kind: ActivityKind::StatusMessage {
                friend: "bob".into(),
                text: "afk".into(),
            },
        },
        bio("bob", FEED_NOW - 10),
        ActivityEntry {
            at: FEED_NOW - 5,
            kind: ActivityKind::NowFriends {
                friend: "bob".into(),
            },
        },
        ActivityEntry {
            at: FEED_NOW - 2,
            kind: ActivityKind::FavouriteAdded {
                game_id: 4181,
                address: "203.0.113.7:27015".into(),
                label: "LAN".into(),
            },
        },
        joined(FEED_NOW - 1),
    ];
    insert(&mut conn, OWNER, &all, FEED_NOW).unwrap();
    let mut newest_first = all.clone();
    newest_first.reverse();
    // The two at FEED_NOW - 40: the later insert reads first.
    assert_eq!(entries(&conn), newest_first);

    let first = page(&conn, OWNER, None, 4).unwrap();
    assert_eq!(first.len(), 4);
    let rest = page(&conn, OWNER, Some(first[3].id), 100).unwrap();
    let paged: Vec<_> = first.iter().chain(&rest).map(|s| s.entry.clone()).collect();
    assert_eq!(paged, newest_first);

    clear(&conn, OWNER).unwrap();
    assert!(page(&conn, OWNER, Some(first[0].id), 10)
        .unwrap()
        .is_empty());
}

/// The bounds: no row older than 30 days before `now` survives an insert
/// (one exactly 30 days old does), and only the newest 500 are kept.
#[test]
fn insert_prunes_to_thirty_days_and_five_hundred_rows() {
    let mut conn = fixture::conn();
    insert(
        &mut conn,
        OWNER,
        &[
            joined(FEED_NOW - MAX_AGE_MS - 1),
            joined(FEED_NOW - MAX_AGE_MS),
        ],
        FEED_NOW,
    )
    .unwrap();
    assert_eq!(entries(&conn), [joined(FEED_NOW - MAX_AGE_MS)]);

    let many: Vec<_> = (1..=505).map(|i| joined(FEED_NOW - 1000 + i)).collect();
    insert(&mut conn, OWNER, &many, FEED_NOW).unwrap();
    let kept = entries(&conn);
    assert_eq!(i64::try_from(kept.len()).unwrap(), MAX_ROWS);
    assert_eq!(kept.first(), Some(&joined(FEED_NOW - 1000 + 505)));
    assert_eq!(kept.last(), Some(&joined(FEED_NOW - 1000 + 6)));
}

/// A friend's removal or block purges their rows, ours and other friends'
/// stay; deleting the friend row purges by cascade too; a row about
/// someone who is not a friend is refused.
#[test]
fn purge_by_friend_keeps_the_rest_and_the_friend_row_cascades() {
    let mut conn = fixture::conn();
    friend(&conn, "bob");
    friend(&conn, "carol");
    insert(
        &mut conn,
        OWNER,
        &[
            bio("bob", FEED_NOW - 3),
            bio("carol", FEED_NOW - 2),
            joined(FEED_NOW - 1),
        ],
        FEED_NOW,
    )
    .unwrap();
    purge_friend(&conn, OWNER, "bob").unwrap();
    assert_eq!(
        entries(&conn),
        [joined(FEED_NOW - 1), bio("carol", FEED_NOW - 2)]
    );

    conn.execute("DELETE FROM friends WHERE public_key = 'carol'", [])
        .unwrap();
    assert_eq!(entries(&conn), [joined(FEED_NOW - 1)]);

    assert!(
        insert(&mut conn, OWNER, &[bio("dave", FEED_NOW)], FEED_NOW).is_err(),
        "only a friend's entries are kept"
    );
    assert_eq!(
        entries(&conn),
        [joined(FEED_NOW - 1)],
        "the refused insert left nothing"
    );
}

/// A row whose payload names another kind than its column is corrupt, not
/// read as either.
#[test]
fn a_payload_of_another_kind_is_a_corrupt_row() {
    let conn = fixture::conn();
    conn.execute(
        "INSERT INTO activity_log (owner_key, at, kind, subject_key, payload_json) \
         VALUES (?1, 1, 'nowFriends', NULL, ?2)",
        params![OWNER, serde_json::to_string(&joined(1).kind).unwrap()],
    )
    .unwrap();
    assert!(page(&conn, OWNER, None, 10).is_err());
}
