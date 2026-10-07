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

use crate::db_helpers::db_call;
use crate::state::SharedState;
use rekindle_db::Db;

/// Load communities and channels from `SQLite` into `AppState`, scoped to the given identity.
pub async fn load_communities_from_db(
    pool: &Db,
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

/// Re-merge each community's cached governance state from the warm local
/// cache (`governance_entries_cache`) and install it on the in-memory
/// `CommunityState`.
///
/// The DHT is the source of truth, but the per-login DHT rebuild
/// (`rebuild_governance_from_dht`) is slow and best-effort — until it
/// completes, `governance_state` would otherwise stay `None` and every
/// governance-gated read (expressions, automod, events, threads, game
/// servers) fails with "governance state not loaded". This restores the
/// last-known merged state immediately so the community is usable on
/// open; the background DHT pass later overwrites it with anything newer.
///
/// The cache stores the lossless CRDT merge *input* —
/// `Vec<(PseudonymKey, Vec<GovernanceEntry>)>` — so re-running
/// [`rekindle_governance::merge::merge_with_accepted`] reproduces an identical
/// `GovernanceState` (a denormalized SQLite view would not, since merge
/// is reader-validates and drops entries from unauthorized authors).
///
/// Must run *after* `restore_community_pseudonyms_and_meks`:
/// [`crate::state_helpers::set_governance_state`] reads
/// `cs.my_pseudonym_key` to sync `my_role_ids`. Best-effort —
/// corrupt/empty rows are skipped, never fatal to login.
pub async fn restore_governance_from_cache(
    pool: &Db,
    state: &SharedState,
    owner_key: &str,
) -> Result<(), String> {
    let ok = owner_key.to_string();
    let cached = db_call(pool, move |conn| {
        rekindle_db::repo::governance_cache::load(conn, &ok)
    })
    .await?;

    for (community_id, entries) in cached {
        if entries.is_empty() {
            continue;
        }
        let (gov_state, accepted_clock) = rekindle_governance::merge::merge_with_accepted(&entries);
        crate::state_helpers::observe_governance_lamport(state, &community_id, accepted_clock);
        crate::state_helpers::set_governance_state(state, &community_id, gov_state);
    }
    Ok(())
}
