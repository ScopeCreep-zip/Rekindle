//! Integration tests for community analytics.
//!
//! Each test boots an in-memory SQLite, applies the full migration so
//! the analytics tables (`community_member_leaves`, `voice_session_events`)
//! are present alongside the existing membership/message tables, seeds
//! representative rows, and exercises the per-section aggregators
//! directly.

use rusqlite::Connection;

use super::{activity_by_hour, channel_metrics, growth, member_metrics, storage};

const MIGRATION: &str = include_str!("../../../src-tauri/migrations/001_init.sql");

const NOW_MS: i64 = 1_700_000_000_000;
const SEVEN_DAYS_MS: i64 = 7 * 24 * 60 * 60 * 1000;
const THIRTY_DAYS_MS: i64 = 30 * 24 * 60 * 60 * 1000;

fn open_db() -> Connection {
    let conn = Connection::open_in_memory().expect("open in-memory db");
    conn.execute_batch(MIGRATION).expect("apply migration");
    conn.execute(
        "INSERT INTO identity (id, public_key, created_at) VALUES (1, 'owner_pk', 0)",
        [],
    )
    .expect("seed identity");
    conn.execute(
        "INSERT INTO communities (owner_key, id, name, joined_at) VALUES ('owner_pk', 'c1', 'C1', 0)",
        [],
    )
    .expect("seed community");
    conn
}

fn add_member(conn: &Connection, pseudonym: &str, joined_at: i64) {
    conn.execute(
        "INSERT INTO community_members \
         (owner_key, community_id, pseudonym_key, display_name, role_ids, joined_at) \
         VALUES ('owner_pk', 'c1', ?1, 'name', '[1]', ?2)",
        rusqlite::params![pseudonym, joined_at],
    )
    .expect("insert member");
}

fn add_leave(conn: &Connection, pseudonym: &str, left_at: i64) {
    conn.execute(
        "INSERT INTO community_member_leaves (owner_key, community_id, pseudonym_key, left_at) \
         VALUES ('owner_pk', 'c1', ?1, ?2)",
        rusqlite::params![pseudonym, left_at],
    )
    .expect("insert leave");
}

fn add_channel(conn: &Connection, channel_id: &str) {
    conn.execute(
        "INSERT INTO channels \
         (owner_key, id, community_id, name, channel_type, sort_order, my_sequence) \
         VALUES ('owner_pk', ?1, 'c1', 'general', 'text', 0, 0)",
        rusqlite::params![channel_id],
    )
    .expect("insert channel");
}

fn add_message(conn: &Connection, channel_id: &str, sender: &str, ts: i64) {
    conn.execute(
        "INSERT INTO messages \
         (owner_key, conversation_id, conversation_type, sender_key, body, automod_blurred, timestamp, is_read) \
         VALUES ('owner_pk', ?1, 'channel', ?2, 'msg', 0, ?3, 0)",
        rusqlite::params![channel_id, sender, ts],
    )
    .expect("insert message");
}

fn add_voice_event(conn: &Connection, channel_id: &str, pseudonym: &str, kind: &str, ts: i64) {
    conn.execute(
        "INSERT INTO voice_session_events \
         (owner_key, community_id, channel_id, member_pseudonym, event_type, occurred_at) \
         VALUES ('owner_pk', 'c1', ?1, ?2, ?3, ?4)",
        rusqlite::params![channel_id, pseudonym, kind, ts],
    )
    .expect("insert voice event");
}

#[test]
fn member_metrics_total_and_window_counts() {
    let conn = open_db();
    add_channel(&conn, "ch1");
    add_member(&conn, "p1", NOW_MS - 2 * SEVEN_DAYS_MS);
    add_member(&conn, "p2", NOW_MS - 3 * 24 * 60 * 60 * 1000); // 3d ago
    add_member(&conn, "p3", NOW_MS - SEVEN_DAYS_MS - 1); // just outside 7d
    add_leave(&conn, "p4", NOW_MS - 24 * 60 * 60 * 1000); // 1d ago
    add_message(&conn, "ch1", "p1", NOW_MS - 24 * 60 * 60 * 1000); // active in 7d
    add_message(&conn, "ch1", "p2", NOW_MS - 60 * 60 * 1000); // active in 7d
    add_message(&conn, "ch1", "p3", NOW_MS - 20 * 24 * 60 * 60 * 1000); // active in 30d only

    let m = member_metrics::compute(&conn, "owner_pk", "c1", NOW_MS);
    assert_eq!(m.total_members, 3);
    assert_eq!(m.active_7d, 2);
    assert_eq!(m.active_30d, 3);
    assert_eq!(m.joins_7d, 1, "only p2 joined inside 7d");
    assert_eq!(m.leaves_7d, 1);
}

#[test]
fn member_metrics_retention_30d_window() {
    let conn = open_db();
    add_channel(&conn, "ch1");
    // Two members joined ≥30 days ago
    add_member(&conn, "p1", NOW_MS - THIRTY_DAYS_MS - 1);
    add_member(&conn, "p2", NOW_MS - THIRTY_DAYS_MS - 1);
    // Only p1 has activity in last 7d
    add_message(&conn, "ch1", "p1", NOW_MS - 24 * 60 * 60 * 1000);

    let m = member_metrics::compute(&conn, "owner_pk", "c1", NOW_MS);
    assert!(
        (m.retention_7_of_30 - 0.5).abs() < 0.001,
        "expected 0.5 retention, got {}",
        m.retention_7_of_30
    );
}

#[test]
fn channel_metrics_messages_and_unique_posters() {
    let conn = open_db();
    add_channel(&conn, "ch1");
    add_channel(&conn, "ch2");
    add_message(&conn, "ch1", "p1", NOW_MS - 24 * 60 * 60 * 1000);
    add_message(&conn, "ch1", "p1", NOW_MS - 60 * 60 * 1000);
    add_message(&conn, "ch1", "p2", NOW_MS - 60 * 60 * 1000);
    add_message(&conn, "ch1", "p3", NOW_MS - 30 * 24 * 60 * 60 * 1000); // outside 7d

    let metrics = channel_metrics::compute(&conn, "owner_pk", "c1", NOW_MS).expect("compute");
    let ch1 = metrics.iter().find(|c| c.channel_id == "ch1").unwrap();
    assert_eq!(ch1.messages_7d, 3);
    assert_eq!(ch1.unique_posters_7d, 2);
    let ch2 = metrics.iter().find(|c| c.channel_id == "ch2").unwrap();
    assert_eq!(ch2.messages_7d, 0);
    assert_eq!(ch2.unique_posters_7d, 0);
}

#[test]
fn channel_metrics_peak_voice_concurrent_replays_events() {
    let conn = open_db();
    add_channel(&conn, "ch1");
    // Sequence: p1 join → p2 join → p3 join (peak=3) → p1 leave → p4 join
    add_voice_event(&conn, "ch1", "p1", "join", 1);
    add_voice_event(&conn, "ch1", "p2", "join", 2);
    add_voice_event(&conn, "ch1", "p3", "join", 3);
    add_voice_event(&conn, "ch1", "p1", "leave", 4);
    add_voice_event(&conn, "ch1", "p4", "join", 5);

    let metrics = channel_metrics::compute(&conn, "owner_pk", "c1", NOW_MS).expect("compute");
    let ch1 = metrics.iter().find(|c| c.channel_id == "ch1").unwrap();
    assert_eq!(ch1.peak_concurrent_voice, 3);
}

#[test]
fn channel_metrics_messages_per_day_buckets_correctly() {
    let conn = open_db();
    add_channel(&conn, "ch1");
    let one_day_ms: i64 = 24 * 60 * 60 * 1000;
    let today = NOW_MS - NOW_MS.rem_euclid(one_day_ms);
    let yesterday = today - one_day_ms;
    let two_days_ago = today - 2 * one_day_ms;

    add_message(&conn, "ch1", "p1", today + 100);
    add_message(&conn, "ch1", "p2", today + 200);
    add_message(&conn, "ch1", "p1", yesterday + 50);
    add_message(&conn, "ch1", "p3", two_days_ago + 30);

    let metrics = channel_metrics::compute(&conn, "owner_pk", "c1", NOW_MS).expect("compute");
    let ch1 = metrics.iter().find(|c| c.channel_id == "ch1").unwrap();

    // 30 samples, oldest first.
    assert_eq!(ch1.messages_per_day.samples.len(), 30);
    let by_day: std::collections::HashMap<i64, u32> = ch1
        .messages_per_day
        .samples
        .iter()
        .map(|s| (s.day_unix_ms, s.value))
        .collect();
    assert_eq!(by_day.get(&today).copied(), Some(2));
    assert_eq!(by_day.get(&yesterday).copied(), Some(1));
    assert_eq!(by_day.get(&two_days_ago).copied(), Some(1));

    // Distinct posters per day.
    let posters_by_day: std::collections::HashMap<i64, u32> = ch1
        .unique_posters_per_day
        .samples
        .iter()
        .map(|s| (s.day_unix_ms, s.value))
        .collect();
    assert_eq!(posters_by_day.get(&today).copied(), Some(2));
    assert_eq!(posters_by_day.get(&yesterday).copied(), Some(1));
}

#[test]
fn member_metrics_active_per_day_buckets_correctly() {
    let conn = open_db();
    add_channel(&conn, "ch1");
    let one_day_ms: i64 = 24 * 60 * 60 * 1000;
    let today = NOW_MS - NOW_MS.rem_euclid(one_day_ms);
    let yesterday = today - one_day_ms;

    // p1 + p2 active today; p1 alone active yesterday.
    add_message(&conn, "ch1", "p1", today + 1);
    add_message(&conn, "ch1", "p2", today + 2);
    add_message(&conn, "ch1", "p1", yesterday + 1);

    let m = member_metrics::compute(&conn, "owner_pk", "c1", NOW_MS);
    let by_day: std::collections::HashMap<i64, u32> = m
        .active_per_day
        .samples
        .iter()
        .map(|s| (s.day_unix_ms, s.value))
        .collect();
    assert_eq!(by_day.get(&today).copied(), Some(2));
    assert_eq!(by_day.get(&yesterday).copied(), Some(1));
}

#[test]
fn growth_metrics_returns_30_daily_samples_in_chronological_order() {
    let conn = open_db();
    add_member(&conn, "p1", NOW_MS - 25 * 24 * 60 * 60 * 1000);
    add_member(&conn, "p2", NOW_MS - 5 * 24 * 60 * 60 * 1000);
    add_leave(&conn, "p3", NOW_MS - 24 * 60 * 60 * 1000);

    let g = growth::compute(&conn, "owner_pk", "c1", NOW_MS);
    assert_eq!(g.samples.len(), 30);
    // Samples should be chronologically ordered (oldest → newest).
    let timestamps: Vec<i64> = g.samples.iter().map(|s| s.day_unix_ms).collect();
    let mut sorted = timestamps.clone();
    sorted.sort_unstable();
    assert_eq!(timestamps, sorted, "samples must be in ascending order");
    // Last sample should reflect joined - left = 2 - 1 = 1
    let last = g.samples.last().unwrap();
    assert_eq!(last.member_count, 1);
}

#[test]
fn storage_usage_grows_with_message_volume() {
    let conn = open_db();
    add_channel(&conn, "ch1");
    let baseline = storage::compute(&conn, "owner_pk", "c1");

    for i in 0..100 {
        add_message(
            &conn,
            "ch1",
            "p1",
            NOW_MS + i64::from(i),
        );
    }
    let after = storage::compute(&conn, "owner_pk", "c1");

    assert!(
        after.message_bytes > baseline.message_bytes,
        "message_bytes should grow: {} -> {}",
        baseline.message_bytes,
        after.message_bytes
    );
    assert!(after.total_bytes > baseline.total_bytes);
    // Total must equal the sum of components.
    assert_eq!(
        after.total_bytes,
        after.message_bytes
            + after.thread_message_bytes
            + after.channel_pin_bytes
            + after.read_state_bytes
            + after.voice_event_bytes
            + after.member_leave_bytes
            + after.metadata_bytes,
    );
}

#[test]
fn activity_by_hour_buckets_by_utc_hour() {
    let conn = open_db();
    add_channel(&conn, "ch1");
    let one_day_ms: i64 = 24 * 60 * 60 * 1000;
    let one_hour_ms: i64 = 60 * 60 * 1000;
    let today_midnight = NOW_MS - NOW_MS.rem_euclid(one_day_ms);

    // Three messages at hour-of-day 14, two at hour-of-day 3.
    add_message(&conn, "ch1", "p1", today_midnight + 14 * one_hour_ms + 1);
    add_message(&conn, "ch1", "p2", today_midnight + 14 * one_hour_ms + 2);
    add_message(&conn, "ch1", "p3", today_midnight + 14 * one_hour_ms + 3);
    add_message(&conn, "ch1", "p1", today_midnight + 3 * one_hour_ms);
    add_message(&conn, "ch1", "p2", today_midnight + 3 * one_hour_ms + 1);

    let abh = activity_by_hour::compute(&conn, "owner_pk", "c1", NOW_MS);
    assert_eq!(abh.hour_counts[14], 3);
    assert_eq!(abh.hour_counts[3], 2);
    assert_eq!(abh.peak_hour(), 14);
    // Other hours stay zero.
    assert_eq!(abh.hour_counts[0], 0);
    assert_eq!(abh.hour_counts[23], 0);
}

