//! Phase 23.C — `clear_registry_presence_slot` lifted from
//! `commands/community/legacy/messages.rs`. Looks up the kicked/leaving
//! member's subkey index, derives the slot signing keypair, and writes
//! an empty value to that subkey of the SMPL member registry — used by
//! the lifecycle (member leaves) and moderation (kick) flows.

use crate::db_helpers::db_call;
use crate::state::SharedState;
use crate::state_helpers;
use rekindle_db::Db;

pub async fn clear_registry_presence_slot(
    state: &SharedState,
    pool: &Db,
    community_id: &str,
    pseudonym_key: &str,
) -> Result<(), String> {
    let (registry_key, slot_seed_hex, my_pseudonym, my_subkey_index) = {
        let communities = state.communities.read();
        let community = communities.get(community_id).ok_or("community not found")?;
        (
            community
                .member_registry_key
                .clone()
                .ok_or("no member registry key")?,
            community
                .slot_seed
                .clone()
                .ok_or("no slot seed available")?,
            community.my_pseudonym_key.clone(),
            community.my_subkey_index,
        )
    };

    let subkey_index = if my_pseudonym.as_deref() == Some(pseudonym_key) {
        my_subkey_index.ok_or("no local subkey index")?
    } else {
        let owner_key = state_helpers::current_owner_key(state)?;
        let cid = community_id.to_string();
        let pk = pseudonym_key.to_string();
        db_call(pool, move |conn| {
            rekindle_db::repo::members::slot(conn, &owner_key, &cid, &pk)
        })
        .await?
        .ok_or("member's registry slot is not known yet")?
        .0
    };

    let slot_seed_bytes: [u8; 32] = hex::decode(&slot_seed_hex)
        .map_err(|e| format!("invalid slot seed hex: {e}"))?
        .try_into()
        .map_err(|_| "slot seed must be 32 bytes")?;
    let slot_keypair =
        rekindle_secrets::derive::derive_slot_keypair(&slot_seed_bytes, subkey_index)
            .map_err(|e| format!("slot keypair derivation failed: {e}"))?;
    let writer = crate::services::community::create::slot_signing_to_veilid(&slot_keypair);
    let record_key = registry_key
        .parse::<veilid_core::RecordKey>()
        .map_err(|e| format!("invalid registry key: {e}"))?;
    let outcome = state_helpers::record_pool(state)?
        .write_once(&record_key, subkey_index, Vec::new(), Some(writer))
        .await
        .map_err(|e| format!("registry slot clear failed: {e}"))?;
    if outcome.missed() {
        return Err(format!("registry slot clear not stored ({outcome:?})"));
    }
    Ok(())
}
