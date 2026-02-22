//! Friend persistence helpers.
//!
//! Two-layer module for the `friends` table:
//!
//! - **Conn-level functions** (`update_*`, `delete_friend`): pure `rusqlite`
//!   functions used inside existing `db_call`/`db_fire` closures.
//! - **Fire-and-forget wrappers** (`fire_*`): encapsulate
//!   `owner_key_or_default → clone → db_fire → conn function` for standalone
//!   one-liner call sites.

use std::sync::Arc;

use crate::db::DbPool;
use crate::db_helpers::db_fire;
use crate::state::AppState;
use crate::state_helpers;

// ── Conn-level functions ────────────────────────────────────────────

/// Update a friend's profile DHT record key.
pub fn update_dht_record_key(
    conn: &rusqlite::Connection,
    owner_key: &str,
    public_key: &str,
    value: &str,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE friends SET dht_record_key = ?1 WHERE owner_key = ?2 AND public_key = ?3",
        rusqlite::params![value, owner_key, public_key],
    )?;
    Ok(())
}

/// Update a friend's display name.
pub fn update_display_name(
    conn: &rusqlite::Connection,
    owner_key: &str,
    public_key: &str,
    value: &str,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE friends SET display_name = ?1 WHERE owner_key = ?2 AND public_key = ?3",
        rusqlite::params![value, owner_key, public_key],
    )?;
    Ok(())
}

/// Update a friend's friendship state (e.g. "accepted", "pending\_in").
pub fn update_friendship_state(
    conn: &rusqlite::Connection,
    owner_key: &str,
    public_key: &str,
    value: &str,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE friends SET friendship_state = ?1 WHERE owner_key = ?2 AND public_key = ?3",
        rusqlite::params![value, owner_key, public_key],
    )?;
    Ok(())
}

/// Update a friend's mailbox DHT key.
pub fn update_mailbox_dht_key(
    conn: &rusqlite::Connection,
    owner_key: &str,
    public_key: &str,
    value: &str,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE friends SET mailbox_dht_key = ?1 WHERE owner_key = ?2 AND public_key = ?3",
        rusqlite::params![value, owner_key, public_key],
    )?;
    Ok(())
}

/// Update a friend's last-seen timestamp.
pub fn update_last_seen_at(
    conn: &rusqlite::Connection,
    owner_key: &str,
    public_key: &str,
    timestamp: i64,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE friends SET last_seen_at = ?1 WHERE owner_key = ?2 AND public_key = ?3",
        rusqlite::params![timestamp, owner_key, public_key],
    )?;
    Ok(())
}

/// Move a friend into (or out of) a group.
pub fn update_group_id(
    conn: &rusqlite::Connection,
    owner_key: &str,
    public_key: &str,
    group_id: Option<i64>,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE friends SET group_id = ?1 WHERE owner_key = ?2 AND public_key = ?3",
        rusqlite::params![group_id, owner_key, public_key],
    )?;
    Ok(())
}

/// Update a friend's local conversation DHT key.
pub fn update_local_conversation_key(
    conn: &rusqlite::Connection,
    owner_key: &str,
    public_key: &str,
    key: &str,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE friends SET local_conversation_key = ?1 WHERE owner_key = ?2 AND public_key = ?3",
        rusqlite::params![key, owner_key, public_key],
    )?;
    Ok(())
}

/// COALESCE-update profile DHT key and/or mailbox DHT key (only overwrites non-NULL values).
pub fn update_dht_and_mailbox_keys(
    conn: &rusqlite::Connection,
    owner_key: &str,
    public_key: &str,
    dht_key: Option<&str>,
    mailbox_key: Option<&str>,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE friends SET dht_record_key = COALESCE(?1, dht_record_key), \
         mailbox_dht_key = COALESCE(?2, mailbox_dht_key) \
         WHERE owner_key = ?3 AND public_key = ?4",
        rusqlite::params![dht_key, mailbox_key, owner_key, public_key],
    )?;
    Ok(())
}

/// Delete a friend row.
pub fn delete_friend(
    conn: &rusqlite::Connection,
    owner_key: &str,
    public_key: &str,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "DELETE FROM friends WHERE owner_key = ?1 AND public_key = ?2",
        rusqlite::params![owner_key, public_key],
    )?;
    Ok(())
}

// ── Fire-and-forget wrappers ────────────────────────────────────────

/// Internal helper: fire-and-forget string-column update on the friends table.
///
/// Captures the common `owner_key_or_default → clone → db_fire → conn fn` pattern
/// shared by the string-valued fire wrappers below.
fn fire_str(
    state: &Arc<AppState>,
    pool: &DbPool,
    public_key: &str,
    value: &str,
    label: &'static str,
    updater: fn(&rusqlite::Connection, &str, &str, &str) -> Result<(), rusqlite::Error>,
) {
    let ok = state_helpers::owner_key_or_default(state);
    let pk = public_key.to_string();
    let v = value.to_string();
    db_fire(pool, label, move |conn| updater(conn, &ok, &pk, &v));
}

/// Fire-and-forget: update a friend's profile DHT record key.
pub fn fire_update_dht_record_key(
    state: &Arc<AppState>,
    pool: &DbPool,
    public_key: &str,
    value: &str,
) {
    fire_str(
        state,
        pool,
        public_key,
        value,
        "update friend DHT key",
        update_dht_record_key,
    );
}

/// Fire-and-forget: update a friend's display name.
pub fn fire_update_display_name(
    state: &Arc<AppState>,
    pool: &DbPool,
    public_key: &str,
    value: &str,
) {
    fire_str(
        state,
        pool,
        public_key,
        value,
        "update friend display name",
        update_display_name,
    );
}

/// Fire-and-forget: update a friend's friendship state.
pub fn fire_update_friendship_state(
    state: &Arc<AppState>,
    pool: &DbPool,
    public_key: &str,
    value: &str,
) {
    fire_str(
        state,
        pool,
        public_key,
        value,
        "update friendship state",
        update_friendship_state,
    );
}

/// Fire-and-forget: update a friend's mailbox DHT key.
pub fn fire_update_mailbox_dht_key(
    state: &Arc<AppState>,
    pool: &DbPool,
    public_key: &str,
    value: &str,
) {
    fire_str(
        state,
        pool,
        public_key,
        value,
        "update friend mailbox key",
        update_mailbox_dht_key,
    );
}

/// Fire-and-forget: delete a friend row.
pub fn fire_delete_friend(state: &Arc<AppState>, pool: &DbPool, public_key: &str) {
    let ok = state_helpers::owner_key_or_default(state);
    let pk = public_key.to_string();
    db_fire(pool, "delete friend", move |conn| {
        delete_friend(conn, &ok, &pk)
    });
}

/// Fire-and-forget: update a friend's last-seen timestamp.
pub fn fire_update_last_seen_at(
    state: &Arc<AppState>,
    pool: &DbPool,
    public_key: &str,
    timestamp: i64,
) {
    let ok = state_helpers::owner_key_or_default(state);
    let pk = public_key.to_string();
    db_fire(pool, "update friend last_seen_at", move |conn| {
        update_last_seen_at(conn, &ok, &pk, timestamp)
    });
}
