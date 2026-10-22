//! Phase 23.D.4 — AppState-mutating helpers extracted from
//! `deps_impl.rs`. Each writes back to `state.communities` (parking_lot)
//! and/or queues a SQLite `db_fire` to persist the change.

use rekindle_governance::state::GovernanceState;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::PseudonymKey;

use crate::state_helpers;

use super::GovernanceAdapter;

pub(super) async fn apply_governance_rebuild_result_impl(
    adapter: &GovernanceAdapter,
    community_id: &str,
    gov_state: GovernanceState,
    accepted_clock: u64,
) {
    state_helpers::observe_governance_lamport(&adapter.state, community_id, accepted_clock);
    state_helpers::set_governance_state(&adapter.state, community_id, gov_state);
    let clock =
        state_helpers::governance_clock(&adapter.state, community_id).unwrap_or(accepted_clock);
    if let Err(error) = state_helpers::persist_governance_snapshot_to_sqlite(
        &adapter.state,
        &adapter.pool,
        community_id,
        clock,
    )
    .await
    {
        tracing::warn!(
            community = %community_id,
            %error,
            "failed to persist rebuilt governance snapshot",
        );
    }
}

pub(super) fn persist_governance_entries_cache_impl(
    adapter: &GovernanceAdapter,
    community_id: &str,
    entries: &[(PseudonymKey, Vec<GovernanceEntry>)],
) {
    let Ok(owner_key) = state_helpers::current_owner_key(&adapter.state) else {
        return;
    };
    if owner_key.is_empty() {
        return;
    }
    let cid = community_id.to_string();
    let entries = entries.to_vec();
    let now = rekindle_utils::timestamp_secs().cast_signed();
    crate::db_helpers::db_fire(
        &adapter.pool,
        "persist governance entries cache",
        move |conn| {
            rekindle_db::repo::governance_cache::save(conn, &owner_key, &cid, &entries, now)
        },
    );
}

pub(super) fn apply_recovered_member_state_impl(
    adapter: &GovernanceAdapter,
    community_id: &str,
    subkey_index: u32,
    role_ids: &[u32],
) {
    let mut recovered_subkey = false;
    let role_ids_to_persist = {
        let mut communities = adapter.state.communities.write();
        let Some(cs) = communities.get_mut(community_id) else {
            return;
        };
        if cs.my_subkey_index.is_none() {
            cs.my_subkey_index = Some(subkey_index);
            recovered_subkey = true;
            tracing::info!(
                community = %community_id,
                subkey_index,
                "recovered my_subkey_index from DHT registry",
            );
        }
        if !role_ids.is_empty() && role_ids.len() >= cs.my_role_ids.len() {
            cs.my_role_ids = role_ids.to_vec();
        }
        cs.my_role_ids.clone()
    };

    let owner_key = state_helpers::current_owner_key(&adapter.state).unwrap_or_default();
    let cid = community_id.to_string();
    crate::db_helpers::db_fire(
        &adapter.pool,
        "persist hydrated subkey_index + role_ids",
        move |conn| {
            use rekindle_db::repo::communities::{set, set_my_role_ids, Column};
            if recovered_subkey {
                set(conn, &owner_key, &cid, Column::MySubkeyIndex, subkey_index)?;
            }
            set_my_role_ids(conn, &owner_key, &cid, &role_ids_to_persist)
        },
    );
}

pub(super) fn try_derive_slot_keypair_if_ready_impl(
    adapter: &GovernanceAdapter,
    community_id: &str,
) {
    let should_derive = {
        let communities = adapter.state.communities.read();
        communities.get(community_id).and_then(|cs| {
            if cs.slot_keypair.is_none() {
                cs.slot_seed
                    .as_ref()
                    .and_then(|seed| cs.my_subkey_index.map(|idx| (seed.clone(), idx)))
            } else {
                None
            }
        })
    };
    if let Some((seed, idx)) = should_derive {
        crate::services::community::try_derive_slot_keypair(
            &adapter.state,
            community_id,
            &seed,
            idx,
        );
    }
}

pub(super) fn spawn_text_mek_rotation_for_ban_impl(
    adapter: &GovernanceAdapter,
    community_id: &str,
    banned_pseudonym_hex: &str,
) {
    crate::services::community::spawn_departure_rotations(
        &adapter.app_handle,
        &adapter.state,
        community_id,
        banned_pseudonym_hex,
    );
}

pub(super) fn register_governance_overflow_keys_impl(
    adapter: &GovernanceAdapter,
    community_id: &str,
    keys: &[String],
) {
    if keys.is_empty() {
        return;
    }
    {
        let mut communities = adapter.state.communities.write();
        if let Some(c) = communities.get_mut(community_id) {
            let inventory = &mut c.open_community_records.governance_overflow_keys;
            for key in keys {
                if !inventory.contains(key) {
                    inventory.push(key.clone());
                }
            }
        }
    }
}
