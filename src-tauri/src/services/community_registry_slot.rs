//! Writes to a member's slot of the SMPL member registry on leave and kick.
//!
//! - Leaving: [`write_departure_tombstone`] writes our signed `departed`
//!   row (plan C7.17), as the daemon does. Never empty bytes: the slot
//!   seed is shared, so an unsigned empty payload lets anyone free anyone's
//!   slot (`MemberPresence::departed`).
//! - Kicking: [`clear_registry_presence_slot`] empties the kicked member's
//!   slot. That is the shared-seed write the plan removes in E3.2
//!   ("moderation no longer writes anyone's slot"); until then a kick stays
//!   as it was.

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
    let writer = rekindle_crypto::keypair_text::keypair_text_from_signing(&slot_keypair);
    let outcome = state_helpers::record_pool(state)?
        .write_once(&registry_key, subkey_index, Vec::new(), Some(&writer))
        .await
        .map_err(|e| format!("registry slot clear failed: {e}"))?;
    if outcome.missed() {
        return Err(format!("registry slot clear not stored ({outcome:?})"));
    }
    Ok(())
}
