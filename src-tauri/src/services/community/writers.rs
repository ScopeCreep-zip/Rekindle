//! A community's writer index on the desktop: which member slot each known
//! member writes (plan C7.12).
//!
//! A multi-writer record (a channel or thread record, one subkey per member
//! slot) is read through this index, never by probing every slot: the
//! presence scan records each member's slot in `community_members`, and our
//! own slot is the community's. It also attributes each slot's entries to
//! its pseudonym.

use std::collections::BTreeMap;
use std::sync::Arc;

use rekindle_db::Db;

use crate::db_helpers::db_call;
use crate::state::AppState;
use crate::state_helpers;

/// Slot → pseudonym for every member of `community_id` whose slot is known,
/// ourselves included.
///
/// # Errors
/// No identity is loaded, or the member query failed.
pub(crate) async fn writer_slots(
    state: &Arc<AppState>,
    pool: &Db,
    community_id: &str,
) -> Result<BTreeMap<u32, String>, String> {
    let mut slots = BTreeMap::new();
    if let Some(community) = state.communities.read().get(community_id) {
        if let (Some(slot), Some(pseudonym)) = (
            community.my_subkey_index,
            community.my_pseudonym_key.clone(),
        ) {
            slots.insert(slot, pseudonym);
        }
    }
    let owner_key = state_helpers::current_owner_key(state)?;
    let community_id = community_id.to_string();
    let rows = db_call(pool, move |conn| {
        rekindle_db::repo::members::slots(conn, &owner_key, &community_id)
    })
    .await?;
    for (pseudonym, slot) in rows {
        slots.insert(slot, pseudonym);
    }
    Ok(slots)
}

/// The slots of [`writer_slots`], for a read.
///
/// # Errors
/// As [`writer_slots`].
pub(crate) async fn writer_slot_list(
    state: &Arc<AppState>,
    pool: &Db,
    community_id: &str,
) -> Result<Vec<u32>, String> {
    Ok(writer_slots(state, pool, community_id)
        .await?
        .into_keys()
        .collect())
}
