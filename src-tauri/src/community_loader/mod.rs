//! Phase 23.C — SQLite-DAO layer for login-time loading of friends,
//! communities, and per-community keystore-backed state.
//!
//! Pre-Phase-23, all of this lived inline in `commands/auth.rs`.
//! Per Invariant 7 (services/ is Tauri-runtime glue only), the
//! community/friend DAO logic doesn't belong there either — it
//! belongs in a dedicated SQLite-loader module sibling to the
//! existing `friend_repo`, `message_repo`, `channel_repo`,
//! `audit_repo` single-file modules. The module-dir split here is
//! by responsibility:
//!
//! - `friends` — `load_friends_from_db` (`friends` table → AppState).
//! - `rows` — row DTOs + the seven `load_*_rows` query helpers + the
//!   batched `fetch_community_loader_rows` call.
//! - `assemble` — pure helpers that project the DTO rows into
//!   `CommunityState` / `ChannelInfo` / `RoleDefinition` etc. +
//!   `build_community_state` (the big composer).
//! - `restore` — `restore_community_pseudonyms_and_meks` (Stronghold
//!   reads + AppState writes).
//!
//! Public entry points re-exported here so callers continue to
//! import via `crate::community_loader::*`.

pub mod assemble;
pub mod friends;
pub mod restore;
pub mod rows;

pub use friends::load_friends_from_db;
pub use restore::restore_community_pseudonyms_and_meks;

use crate::db::DbPool;
use crate::state::SharedState;

/// Load communities and channels from `SQLite` into `AppState`, scoped to the given identity.
pub async fn load_communities_from_db(
    pool: &DbPool,
    state: &SharedState,
    owner_key: &str,
) -> Result<(), String> {
    let rows = rows::fetch_community_loader_rows(pool, owner_key).await?;
    let mut communities = state.communities.write();
    for community in &rows.communities {
        let community_state = assemble::build_community_state(community, &rows);
        communities.insert(community.id.clone(), community_state);
    }
    Ok(())
}
