//! Phase 13 — thin facade over `rekindle_dm::SqliteDmStore`.
//!
//! Pre-Phase-13 this file held the SQLite CRUD + types directly. Now
//! the CRUD lives in `rekindle-dm::store` and this file is the
//! src-tauri adapter: extracts `owner_key` from `AppState` via
//! `state_helpers`, wraps the `DbPool` in a `SqliteDmStore`, and
//! delegates each public function to the trait method.
//!
//! The pub-use re-exports preserve the prior type aliases at
//! `crate::services::dm::store::{DmConversation, DmMessageRecord}` so
//! call sites in `commands/dm.rs` + `services/dm/{ingest,messages,create}.rs`
//! continue to compile without import changes.

use std::sync::Arc;

use rekindle_dm::{DmStore, SqliteDmStore};

use crate::db::DbPool;
use crate::state::AppState;
use crate::state_helpers;

// Re-export the moved types so existing call sites keep their imports.
pub use rekindle_dm::{DmConversation, DmMessageRecord};

pub async fn list_dm_conversations(state: &Arc<AppState>, pool: &DbPool) -> Vec<DmConversation> {
    let owner_key = state_helpers::owner_key_or_default(state);
    if owner_key.is_empty() {
        return Vec::new();
    }
    let store = SqliteDmStore::new(pool.clone());
    store
        .list_conversations(&owner_key)
        .await
        .unwrap_or_default()
}

pub async fn load_dm_messages(
    state: &Arc<AppState>,
    pool: &DbPool,
    record_key: &str,
    limit: i64,
) -> Vec<DmMessageRecord> {
    let owner_key = state_helpers::owner_key_or_default(state);
    if owner_key.is_empty() {
        return Vec::new();
    }
    let store = SqliteDmStore::new(pool.clone());
    store
        .load_messages(&owner_key, record_key, limit)
        .await
        .unwrap_or_default()
}

pub async fn decline_dm_invite(
    state: &Arc<AppState>,
    pool: &DbPool,
    record_key: &str,
) -> Result<(), String> {
    let owner_key = state_helpers::owner_key_or_default(state);
    if owner_key.is_empty() {
        return Err("no identity".into());
    }
    let store = SqliteDmStore::new(pool.clone());
    store
        .decline_invite(&owner_key, record_key)
        .await
        .map_err(|e| e.to_string())
}
