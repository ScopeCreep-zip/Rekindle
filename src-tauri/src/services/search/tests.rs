//! Integration tests for the FTS5 search pipeline.
//!
//! Each test opens an in-memory SQLite, applies the full `001_init.sql`
//! migration so the FTS5 virtual tables and triggers are present, seeds
//! representative rows, and runs the per-table search functions
//! directly. This proves the SQL strings, the FTS5 MATCH grammar, and
//! the bm25() ranking all work together against the real schema —
//! mocking the schema would defeat the point.

use rekindle_types::search::{
    HasFilter, MessageSearch, SearchFilters, SearchSort,
};
use rusqlite::Connection;

use super::messages::search_messages_table;
use super::query::build_match_expr;

const MIGRATION: &str = include_str!("../../../migrations/001_init.sql");

fn open_test_db() -> Connection {
    let conn = Connection::open_in_memory().expect("open in-memory db");
    conn.execute_batch(MIGRATION).expect("apply migration");
    conn.execute(
        "INSERT INTO identity (id, public_key, created_at) VALUES (1, 'owner_pk', 0)",
        [],
    )
    .expect("seed identity");
    conn
}

fn insert_message(
    conn: &Connection,
    conversation_id: &str,
    sender: &str,
    body: &str,
    timestamp: i64,
    attachment_json: Option<&str>,
    message_id: Option<&str>,
) {
    conn.execute(
        "INSERT INTO messages \
         (owner_key, conversation_id, conversation_type, sender_key, body, \
          automod_blurred, timestamp, is_read, mek_generation, message_id, attachment_json) \
         VALUES ('owner_pk', ?1, 'channel', ?2, ?3, 0, ?4, 0, 0, ?5, ?6)",
        rusqlite::params![conversation_id, sender, body, timestamp, message_id, attachment_json],
    )
    .expect("insert message");
}

fn default_search(query: &str) -> MessageSearch {
    MessageSearch {
        query: query.to_string(),
        filters: SearchFilters::default(),
        sort: SearchSort::Relevance,
        limit: 25,
        offset: 0,
    }
}

#[test]
fn fts5_finds_simple_word_match() {
    let conn = open_test_db();
    insert_message(&conn, "ch1", "alice", "the lost cargo arrived", 100, None, Some("m1"));
    insert_message(&conn, "ch1", "bob", "no relevant text here", 200, None, Some("m2"));

    let req = default_search("cargo");
    let match_expr = build_match_expr(&req.query).expect("build match");
    let hits = search_messages_table(&conn, "owner_pk", &match_expr, &req).expect("search");

    assert_eq!(hits.len(), 1, "only one row contains 'cargo'");
    assert_eq!(hits[0].body, "the lost cargo arrived");
    assert_eq!(hits[0].sender_key, "alice");
}

#[test]
fn fts5_returns_adjacent_context() {
    let conn = open_test_db();
    insert_message(&conn, "ch1", "alice", "before the hit", 100, None, Some("m1"));
    insert_message(&conn, "ch1", "alice", "match here cargo", 200, None, Some("m2"));
    insert_message(&conn, "ch1", "alice", "after the hit", 300, None, Some("m3"));

    let req = default_search("cargo");
    let match_expr = build_match_expr(&req.query).unwrap();
    let hits = search_messages_table(&conn, "owner_pk", &match_expr, &req).unwrap();

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].before_body.as_deref(), Some("before the hit"));
    assert_eq!(hits[0].after_body.as_deref(), Some("after the hit"));
}

#[test]
fn channel_filter_restricts_results() {
    let conn = open_test_db();
    insert_message(&conn, "ch1", "alice", "shared keyword here", 100, None, Some("m1"));
    insert_message(&conn, "ch2", "bob", "shared keyword again", 200, None, Some("m2"));

    let mut req = default_search("keyword");
    req.filters.in_channel = Some("ch1".to_string());
    let match_expr = build_match_expr(&req.query).unwrap();
    let hits = search_messages_table(&conn, "owner_pk", &match_expr, &req).unwrap();

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].conversation_id, "ch1");
}

#[test]
fn sender_filter_restricts_results() {
    let conn = open_test_db();
    insert_message(&conn, "ch1", "alice", "found by both", 100, None, Some("m1"));
    insert_message(&conn, "ch1", "bob", "found by both", 200, None, Some("m2"));

    let mut req = default_search("found");
    req.filters.from = Some("bob".to_string());
    let match_expr = build_match_expr(&req.query).unwrap();
    let hits = search_messages_table(&conn, "owner_pk", &match_expr, &req).unwrap();

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].sender_key, "bob");
}

#[test]
fn time_window_filter_restricts_results() {
    let conn = open_test_db();
    insert_message(&conn, "ch1", "alice", "old news here", 100, None, Some("m1"));
    insert_message(&conn, "ch1", "alice", "recent news here", 1000, None, Some("m2"));

    let mut req = default_search("news");
    req.filters.after = Some(500);
    let match_expr = build_match_expr(&req.query).unwrap();
    let hits = search_messages_table(&conn, "owner_pk", &match_expr, &req).unwrap();

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].timestamp, 1000);
}

#[test]
fn has_filter_restricts_to_attachment_kind() {
    let conn = open_test_db();
    insert_message(
        &conn,
        "ch1",
        "alice",
        "look at this picture",
        100,
        Some(r#"{"kind":"image","ref":"abc"}"#),
        Some("m1"),
    );
    insert_message(&conn, "ch1", "alice", "look at this picture", 200, None, Some("m2"));

    let mut req = default_search("picture");
    req.filters.has = vec![HasFilter::Image];
    let match_expr = build_match_expr(&req.query).unwrap();
    let hits = search_messages_table(&conn, "owner_pk", &match_expr, &req).unwrap();

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].timestamp, 100);
}

#[test]
fn fts5_triggers_keep_index_in_sync_on_delete() {
    let conn = open_test_db();
    insert_message(&conn, "ch1", "alice", "delete-me-test", 100, None, Some("m1"));
    let req = default_search("delete-me-test");
    let match_expr = build_match_expr(&req.query).unwrap();
    let pre = search_messages_table(&conn, "owner_pk", &match_expr, &req).unwrap();
    assert_eq!(pre.len(), 1);

    conn.execute("DELETE FROM messages WHERE message_id = 'm1'", [])
        .expect("delete");
    let post = search_messages_table(&conn, "owner_pk", &match_expr, &req).unwrap();
    assert!(post.is_empty(), "FTS5 trigger should remove deleted rows");
}

#[test]
fn fts5_search_perf_5k_messages_within_budget() {
    // Architecture §32 / Week 26: FTS5 search must stay snappy on
    // realistic local message volumes. 5,000 messages is the order of
    // magnitude a single channel reaches in a busy week. Spec doesn't
    // pin a hard FTS5 number but the inspect-poll cycle assumes
    // <250ms search; we guard at 500ms (debug mode) so an accidental
    // regression like "scan all rows in Rust" trips this.
    let conn = open_test_db();
    for i in 0..5_000 {
        let body = format!("message {i} content needle{}", i % 100);
        insert_message(
            &conn,
            "ch1",
            "alice",
            &body,
            i64::from(i),
            None,
            Some(&format!("m{i}")),
        );
    }

    let req = default_search("needle42");
    let match_expr = build_match_expr(&req.query).unwrap();
    let started = std::time::Instant::now();
    let hits = search_messages_table(&conn, "owner_pk", &match_expr, &req).unwrap();
    let elapsed = started.elapsed();

    assert!(!hits.is_empty(), "should find at least one row");
    assert!(
        elapsed.as_millis() < 500,
        "FTS5 search over 5k rows took {elapsed:?} — exceeds 500ms regression budget"
    );
    tracing::info!(?elapsed, "[perf] search(5k rows, needle42)");
}

#[test]
fn ranking_orders_relevance_by_bm25() {
    let conn = open_test_db();
    insert_message(
        &conn,
        "ch1",
        "alice",
        "rare keyword once",
        100,
        None,
        Some("m1"),
    );
    insert_message(
        &conn,
        "ch1",
        "alice",
        "rare keyword keyword keyword many times",
        200,
        None,
        Some("m2"),
    );

    let req = default_search("keyword");
    let match_expr = build_match_expr(&req.query).unwrap();
    let hits = search_messages_table(&conn, "owner_pk", &match_expr, &req).unwrap();

    assert_eq!(hits.len(), 2);
    // Higher rank = better match — m2 has 3 occurrences, should outrank m1.
    assert!(
        hits[0].rank > hits[1].rank,
        "m2 (3 hits) should outrank m1 (1 hit): {} vs {}",
        hits[0].rank,
        hits[1].rank
    );
    assert_eq!(hits[0].timestamp, 200);
}
