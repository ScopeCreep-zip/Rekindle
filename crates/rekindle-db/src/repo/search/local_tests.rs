//! Local search (plan E7.2 G11) against the real schema: each scope finds
//! only its own kind, and a conversation's messages only its own.

use rekindle_types::dto::local_search::{LocalHit, LocalSearchScope};
use rekindle_types::search::SearchScope;
use rusqlite::Connection;

use super::local_search;

const SEARCHER: &str = "owner_pk";

fn seed(conn: &Connection) {
    conn.execute_batch(&format!(
        "INSERT INTO friends (owner_key, public_key, display_name, added_at, friendship_state) \
           VALUES ('{SEARCHER}', 'pk_alice', 'Alice', 1, 'accepted'), \
                  ('{SEARCHER}', 'pk_alfred', 'Alfred', 1, 'pending_out'); \
         INSERT INTO friend_nicknames (owner_key, contact_key, for_key, nickname) \
           VALUES ('{SEARCHER}', 'pk_alice', 'pk_alice', 'Ally'); \
         INSERT INTO group_dms (owner_key, group_id, name, roster, epoch, created_at) \
           VALUES ('{SEARCHER}', '{g}', 'Ally raid', '[]', 0, 1); \
         INSERT INTO favourite_servers (owner_key, game_id, address, label, added_at) \
           VALUES ('{SEARCHER}', 4100, '203.0.113.7:27015', 'Ally clan', 1); \
         INSERT INTO recent_servers (owner_key, game_id, address, last_seen) \
           VALUES ('{SEARCHER}', 4100, '198.51.100.2:27015', 5); \
         INSERT INTO transfers (owner_key, attachment_id, target_json, direction, state, \
                                dest_name, dest_path, started_at) \
           VALUES ('{SEARCHER}', 'att-1', \
                   '{{\"kind\":\"channel\",\"community\":\"c\",\"channelId\":\"ch\"}}', \
                   'download', 'done', 'ally_map.zip', '/x/ally_map.zip', 1);",
        g = "0a".repeat(16),
    ))
    .expect("seed");
    for (conversation, kind, body, ts) in [
        ("pk_alice", "dm", "meet at nine", 10),
        ("pk_bob", "dm", "meet at ten", 11),
        ("ch1", "channel", "meet in the channel", 12),
    ] {
        conn.execute(
            "INSERT INTO messages (owner_key, conversation_id, conversation_type, sender_key, \
                                   body, timestamp) VALUES (?1, ?2, ?3, 'pk_alice', ?4, ?5)",
            rusqlite::params![SEARCHER, conversation, kind, body, ts],
        )
        .expect("insert message");
    }
}

fn run(scope: &LocalSearchScope, query: &str) -> Vec<LocalHit> {
    let conn = crate::test_support::schema_conn(SEARCHER);
    seed(&conn);
    local_search(&conn, SEARCHER, scope, query).expect("local search")
}

#[test]
fn friends_scope_finds_accepted_friends_groups_only() {
    let hits = run(&LocalSearchScope::Friends, "al");
    let kinds: Vec<String> = hits
        .iter()
        .map(|h| match h {
            LocalHit::Friend { public_key, name } => format!("friend {public_key} {name}"),
            LocalHit::GroupDm { name, .. } => format!("group {name}"),
            other => format!("other {other:?}"),
        })
        .collect();
    // Alfred is only pending; the servers and files never show here.
    assert_eq!(kinds, ["friend pk_alice Ally", "group Ally raid"]);
}

#[test]
fn servers_scope_finds_favourites_then_recent() {
    let hits = run(&LocalSearchScope::Servers, "27015");
    let rows: Vec<(String, String)> = hits
        .into_iter()
        .map(|h| match h {
            LocalHit::Server { address, label, .. } => (address, label),
            other => panic!("not a server: {other:?}"),
        })
        .collect();
    assert_eq!(
        rows,
        [
            ("203.0.113.7:27015".to_owned(), "Ally clan".to_owned()),
            ("198.51.100.2:27015".to_owned(), String::new()),
        ]
    );
}

#[test]
fn files_scope_finds_held_attachments() {
    let hits = run(&LocalSearchScope::Files, "ally");
    assert!(
        matches!(hits.as_slice(), [LocalHit::File { attachment_id, name, .. }]
            if attachment_id == "att-1" && name == "ally_map.zip"),
        "{hits:?}"
    );
}

#[test]
fn conversation_scope_finds_only_that_conversations_messages() {
    let scope = LocalSearchScope::Conversation {
        conversation_id: "pk_alice".into(),
    };
    let hits = run(&scope, "meet");
    assert!(
        matches!(hits.as_slice(), [LocalHit::Message(hit)]
            if hit.conversation_id == "pk_alice" && hit.scope == SearchScope::Dm
                && hit.body == "meet at nine"),
        "{hits:?}"
    );
}

#[test]
fn like_wildcards_are_literal_and_blank_finds_nothing() {
    assert!(run(&LocalSearchScope::Friends, "%").is_empty());
    assert!(run(&LocalSearchScope::Friends, "   ").is_empty());
}
