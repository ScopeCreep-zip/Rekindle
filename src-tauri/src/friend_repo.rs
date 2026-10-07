//! Fire-and-forget writes to the `friends` table for one-line call sites:
//! the owner key comes from state and the SQL from
//! `rekindle_db::repo::friends`.

use std::sync::Arc;

use rekindle_db::repo::friends::{self, Column};

use crate::db_helpers::db_fire;
use crate::state::AppState;
use crate::state_helpers;
use rekindle_db::Db;

/// Set one column of a friend's row in the background.
fn fire_set<V>(
    state: &Arc<AppState>,
    pool: &Db,
    public_key: &str,
    column: Column,
    value: V,
    label: &'static str,
) where
    V: rusqlite::ToSql + Send + 'static,
{
    let ok = state_helpers::owner_key_or_default(state);
    let pk = public_key.to_string();
    db_fire(pool, label, move |conn| {
        friends::set(conn, &ok, &pk, column, value)
    });
}

/// Fire-and-forget: update a friend's profile DHT record key.
pub fn fire_update_dht_record_key(state: &Arc<AppState>, pool: &Db, public_key: &str, value: &str) {
    fire_set(
        state,
        pool,
        public_key,
        Column::DhtRecordKey,
        value.to_string(),
        "update friend DHT key",
    );
}

/// Fire-and-forget: update a friend's display name.
pub fn fire_update_display_name(state: &Arc<AppState>, pool: &Db, public_key: &str, value: &str) {
    fire_set(
        state,
        pool,
        public_key,
        Column::DisplayName,
        value.to_string(),
        "update friend display name",
    );
}

/// Fire-and-forget: update a friend's friendship state.
pub fn fire_update_friendship_state(
    state: &Arc<AppState>,
    pool: &Db,
    public_key: &str,
    value: &str,
) {
    fire_set(
        state,
        pool,
        public_key,
        Column::FriendshipState,
        value.to_string(),
        "update friendship state",
    );
}

/// Fire-and-forget: update a friend's mailbox DHT key.
pub fn fire_update_mailbox_dht_key(
    state: &Arc<AppState>,
    pool: &Db,
    public_key: &str,
    value: &str,
) {
    fire_set(
        state,
        pool,
        public_key,
        Column::MailboxDhtKey,
        value.to_string(),
        "update friend mailbox key",
    );
}

/// Fire-and-forget: update a friend's last-seen timestamp.
pub fn fire_update_last_seen_at(
    state: &Arc<AppState>,
    pool: &Db,
    public_key: &str,
    timestamp: i64,
) {
    fire_set(
        state,
        pool,
        public_key,
        Column::LastSeenAt,
        timestamp,
        "update friend last_seen_at",
    );
}

/// Fire-and-forget: delete a friend row.
pub fn fire_delete_friend(state: &Arc<AppState>, pool: &Db, public_key: &str) {
    let ok = state_helpers::owner_key_or_default(state);
    let pk = public_key.to_string();
    db_fire(pool, "delete friend", move |conn| {
        friends::delete(conn, &ok, &pk)
    });
}
