//! Phase 23.D.4 — AppState-mutating helpers extracted from
//! `deps_impl.rs`. Each writes back to `state.communities` (parking_lot)
//! and/or queues a SQLite `db_fire` to persist the change.

use std::collections::HashMap;

use rekindle_governance::state::GovernanceState;
use rekindle_governance_runtime::DiscoveredMember;
use rekindle_types::presence::MemberPresence;

use crate::state_helpers;

use super::GovernanceAdapter;

pub(super) async fn apply_governance_rebuild_result_impl(
    adapter: &GovernanceAdapter,
    community_id: &str,
    gov_state: GovernanceState,
    max_lamport: u64,
) {
    {
        let mut communities = adapter.state.communities.write();
        if let Some(cs) = communities.get_mut(community_id) {
            cs.lamport_counter = cs.lamport_counter.max(max_lamport);
        }
    }
    state_helpers::set_governance_state(&adapter.state, community_id, gov_state);
    if let Err(error) = state_helpers::persist_governance_snapshot_to_sqlite(
        &adapter.state,
        &adapter.pool,
        community_id,
        max_lamport,
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
    let roles_json = serde_json::to_string(&role_ids_to_persist).ok();
    let idx = subkey_index;
    crate::db_helpers::db_fire(
        &adapter.pool,
        "persist hydrated subkey_index + role_ids",
        move |conn| {
            if recovered_subkey {
                conn.execute(
                    "UPDATE communities SET my_subkey_index = ?1 WHERE owner_key = ?2 AND id = ?3",
                    rusqlite::params![idx, &owner_key, &cid],
                )?;
            }
            if let Some(rj) = roles_json {
                conn.execute(
                    "UPDATE communities SET my_role_ids = ?1 WHERE owner_key = ?2 AND id = ?3",
                    rusqlite::params![rj, &owner_key, &cid],
                )?;
            }
            Ok(())
        },
    );
}

pub(super) fn persist_discovered_registry_members_impl(
    adapter: &GovernanceAdapter,
    community_id: &str,
    members: Vec<DiscoveredMember>,
) {
    let mut member_roles: HashMap<String, Vec<u32>> = HashMap::new();
    let mut rows: Vec<(u32, u32, MemberPresence)> = Vec::with_capacity(members.len());
    for m in members {
        let pseudonym_hex = hex::encode(m.presence.pseudonym_key.0);
        member_roles.insert(pseudonym_hex, m.role_ids.clone());
        rows.push((m.segment_index, m.slot_index, m.presence));
    }
    crate::services::community::presence::registry::persist_discovered_registry_members(
        &adapter.state,
        community_id,
        &rows,
        &member_roles,
        &std::collections::HashSet::new(),
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

pub(super) fn recover_registry_keypair_from_keystore_impl(
    adapter: &GovernanceAdapter,
    community_id: &str,
) {
    let ks_guard = adapter.state.keystore.lock();
    let Some(ref ks) = *ks_guard else { return };
    let Some(rkp) = crate::keystore::load_registry_keypair(ks, community_id) else {
        return;
    };
    tracing::info!(
        community = %community_id,
        "recovered registry_owner_keypair from Stronghold during hydrate",
    );
    let mut communities = adapter.state.communities.write();
    if let Some(cs) = communities.get_mut(community_id) {
        cs.registry_owner_keypair = Some(rkp);
    }
}

pub(super) fn mark_community_records_open_impl(
    adapter: &GovernanceAdapter,
    community_id: &str,
    governance_key: &str,
    registry_key: Option<&str>,
    registry_writer: Option<&str>,
    channel_keys: Vec<String>,
) {
    let mut cs = adapter.state.communities.write();
    if let Some(c) = cs.get_mut(community_id) {
        c.open_community_records.governance_key = Some(governance_key.to_string());
        c.open_community_records.registry_key = registry_key.map(str::to_string);
        c.open_community_records.registry_writer = registry_writer.map(str::to_string);
        c.open_community_records.channel_keys = channel_keys;
        c.open_community_records.records_open = true;
    }
}

pub(super) fn spawn_text_mek_rotation_for_ban_impl(
    adapter: &GovernanceAdapter,
    community_id: &str,
    banned_pseudonym_hex: &str,
) {
    let state = adapter.state.clone();
    let app_handle = adapter.app_handle.clone();
    let community_id = community_id.to_string();
    let banned_pseudonym = banned_pseudonym_hex.to_string();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = crate::services::community::rotate_text_mek_for_departure(
            &app_handle,
            &state,
            &community_id,
            &banned_pseudonym,
        )
        .await
        {
            tracing::debug!(
                community = %community_id,
                member = %banned_pseudonym,
                %error,
                "text MEK rotation skipped after governance ban sync",
            );
        }
    });
}
