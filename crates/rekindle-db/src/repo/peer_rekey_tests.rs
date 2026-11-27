use super::*;
use crate::repo::fixture;

const OWNER: [u8; 32] = [1; 32];
const OLD: [u8; 32] = [2; 32];
const NEW: [u8; 32] = [3; 32];

#[test]
fn a_rotated_friend_moves_whole_and_their_old_session_goes() {
    let conn = fixture::identity_conn(&OWNER);
    let (owner, old) = (identity_text(&OWNER), identity_text(&OLD));
    conn.execute(
        "INSERT INTO friends (owner_key, public_key, display_name, added_at, friendship_state, \
             mailbox_dht_key) VALUES (?1, ?2, 'Bob', 1, 'accepted', 'VLD0:old-mailbox')",
        params![owner, old],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO messages (owner_key, conversation_id, conversation_type, sender_key, \
             body, timestamp) VALUES (?1, ?2, 'dm', ?2, 'hi', 1)",
        params![owner, old],
    )
    .unwrap();
    super::super::outbox::queue_for_test(&conn, &owner, &format!("dm:{old}"), "m1", b"{}");
    conn.execute(
        "INSERT INTO ratchet_sessions (owner_key, peer_identity, peer_device, record, updated_at) \
             VALUES (?1, ?2, '', x'00', 1)",
        params![owner, old],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO missed_calls (call_id, owner_key, peer_key, kind, expired_at) \
             VALUES ('c1', ?1, ?2, 0, 1)",
        params![owner, old],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO strand_relay_volunteered (owner_key, friend_public_key, relay_route_id, \
             relay_route_blob, created_at) VALUES (?1, ?2, 'r', x'00', 1)",
        params![owner, old],
    )
    .unwrap();
    for sql in [
        "INSERT INTO stream_cursors (owner_key, stream, writer, seq, updated_at) \
             VALUES (?1, 'dm', ?2, 4, 1)",
        "INSERT INTO stream_applied (owner_key, stream, writer, seq) VALUES (?1, 'dm', ?2, 6)",
        "INSERT INTO received (owner_key, stream, client_id) VALUES (?1, 'dm:' || ?2, 'c')",
        "INSERT INTO chain_entries (owner_key, peer, chain_seq, stream_seq, appended_at) \
             VALUES (?1, ?2, 1, 1, 1)",
    ] {
        conn.execute(sql, params![owner, old]).unwrap();
    }
    signal_identities::ensure(&conn, &OWNER, &OLD).unwrap();
    signal_identities::mark_verified(&conn, &OWNER, &OLD, 5).unwrap();
    // Rows keyed by the friend row (foreign keys to it).
    for sql in [
        "INSERT INTO friend_game_totals (owner_key, friend_key, game_id, total_secs, \
             last_played) VALUES (?1, ?2, 7, 60, 1)",
        "INSERT INTO activity_log (owner_key, at, kind, subject_key, payload_json) \
             VALUES (?1, 1, 'nowFriends', ?2, '{}')",
        "INSERT INTO activity_seen (owner_key, friend_key, accepted, display_name) \
             VALUES (?1, ?2, 1, 'Bob')",
        "INSERT INTO read_markers (owner_key, scope_kind, peer_key, last_read_seq, \
             updated_at) VALUES (?1, 'dm', ?2, 3, 1)",
    ] {
        conn.execute(sql, params![owner, old]).unwrap();
    }

    assert!(rotate_peer(&conn, &OWNER, &OLD, &NEW, 9).unwrap());
    let count = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap() };
    let new = identity_text(&NEW);
    assert_eq!(
        count(&format!("SELECT COUNT(*) FROM messages WHERE conversation_id = '{new}' AND sender_key = '{new}'")),
        1
    );
    assert_eq!(
        count(&format!(
            "SELECT COUNT(*) FROM friends WHERE public_key = '{new}'"
        )),
        1
    );
    assert_eq!(
        count("SELECT COUNT(*) FROM friends WHERE mailbox_dht_key IS NOT NULL"),
        0
    );
    assert_eq!(count("SELECT COUNT(*) FROM outbox"), 0);
    assert_eq!(
        count(&format!(
            "SELECT COUNT(*) FROM missed_calls WHERE peer_key = '{new}'"
        )),
        1
    );
    assert_eq!(
        count(&format!(
            "SELECT COUNT(*) FROM strand_relay_volunteered WHERE friend_public_key = '{new}'"
        )),
        1
    );
    assert_eq!(count("SELECT COUNT(*) FROM ratchet_sessions"), 0);
    // Their chain's cursor and applied ids move; our chain's entries go.
    assert_eq!(
        crate::repo::stream_cursors::get(&conn, &owner, "dm", &new).unwrap(),
        4
    );
    assert_eq!(
        count(&format!(
            "SELECT COUNT(*) FROM stream_applied WHERE writer = '{new}' AND seq = 6"
        )),
        1
    );
    assert_eq!(
        count(&format!(
            "SELECT COUNT(*) FROM received WHERE stream = 'dm:{new}'"
        )),
        1
    );
    assert_eq!(
        count(&format!(
            "SELECT COUNT(*) FROM outbox_streams WHERE stream = 'dm:{new}'"
        )),
        1
    );
    assert_eq!(count("SELECT COUNT(*) FROM chain_entries"), 0);
    // What is keyed by their friend row moves with it.
    for sql in [
        "SELECT COUNT(*) FROM friend_game_totals WHERE friend_key = '{new}'",
        "SELECT COUNT(*) FROM activity_log WHERE subject_key = '{new}'",
        "SELECT COUNT(*) FROM activity_seen WHERE friend_key = '{new}'",
        "SELECT COUNT(*) FROM read_markers WHERE peer_key = '{new}'",
    ] {
        assert_eq!(count(&sql.replace("{new}", &new)), 1, "{sql}");
    }
    // The verified key changed: the new row is unverified and records it.
    let row = signal_identities::get(&conn, &OWNER, &NEW)
        .unwrap()
        .unwrap();
    assert!(!row.verified);
    assert_eq!(row.changed_from_key, Some(OLD));
    assert!(signal_identities::get(&conn, &OWNER, &OLD)
        .unwrap()
        .is_none());
    assert!(!rotate_peer(&conn, &OWNER, &[9; 32], &[8; 32], 9).unwrap());
}
