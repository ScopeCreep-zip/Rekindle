//! Phase 23.C — community-lifecycle runtime orchestration lifted from
//! `commands/community/crud.rs`. Same pattern as the other
//! `services/community_*_runtime.rs` modules.
//!
//! `leave_community_inner` consolidates the post-leave teardown:
//! gossip the `MemberLeave` envelope, clear the local registry slot,
//! close the community's DHT records, wipe the MEK + slot/registry
//! keystore entries, drop the `CommunityState` from AppState, log
//! analytics, delete the SQLite row, and append a `ChannelLeft` audit
//! entry. Pre-Phase-23 this was a ~110-LoC inline body in the
//! `leave_community` Tauri command.

use crate::db_helpers::db_call;
use crate::state::SharedState;
use crate::state_helpers;
use rekindle_db::Db;
use rekindle_types::channel_keys::KeyScope;
use rekindle_vault::VaultHandle;

/// How long a left community's tasks get to stop.
const COMMUNITY_STOP_DEADLINE: std::time::Duration = rekindle_types::config::SESSION_STOP_DEADLINE;

pub async fn leave_community_inner(
    state: &SharedState,
    pool: &Db,
    keystore_handle: &VaultHandle,
    community_id: &str,
) -> Result<(), String> {
    use crate::services::community_registry_slot::write_departure_tombstone;

    let my_pseudonym_key = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .and_then(|community| community.my_pseudonym_key.clone())
            .unwrap_or_default()
    };
    // The community's loops stop first, so none writes to a record this
    // leave is about to clear or close. The closed scope stays on the
    // community until it is removed below, refusing late spawns.
    let tasks = state
        .communities
        .read()
        .get(community_id)
        .and_then(|community| community.tasks.clone());
    if let Some(tasks) = tasks {
        if let Err(stuck) = tasks.shutdown(COMMUNITY_STOP_DEADLINE).await {
            tracing::warn!(community = %community_id, %stuck, "community tasks did not stop in time");
        }
    }
    let _ = crate::services::community::send_to_mesh(
        state,
        community_id,
        &rekindle_codec::community::envelope::CommunityEnvelope::Control(
            rekindle_codec::community::envelope::ControlPayload::MemberLeave {
                pseudonym_key: my_pseudonym_key.clone(),
            },
        ),
    );

    if let Err(error) = write_departure_tombstone(state, community_id).await {
        tracing::warn!(
            community = %community_id,
            error = %error,
            "departure tombstone not written; the slot stays ours until reclaimed",
        );
    }

    // The community's records go back to the pool; the last borrower's
    // release closes each one (plan C7.5).
    crate::services::community::leases::release_all(state, community_id).await;

    // Every key of every scope goes, live and stored: a member who left
    // keeps nothing that opens the community's past or future traffic.
    let scopes = state_helpers::forget_community_keys(state, community_id);

    {
        let ks = keystore_handle.lock();
        if let Some(ref keystore) = *ks {
            for scope in scopes {
                crate::keystore::delete_scope_meks(keystore, community_id, scope);
            }
            crate::keystore::delete_slot_keypair(keystore, community_id);
            crate::keystore::delete_slot_seed(keystore, community_id);
            crate::keystore::delete_registry_keypair(keystore, community_id);
        }
    }

    state.communities.write().remove(community_id);

    let owner_key = state_helpers::current_owner_key(state)?;
    if !my_pseudonym_key.is_empty() {
        crate::services::community::analytics::log_member_leave(
            pool,
            &owner_key,
            community_id,
            &my_pseudonym_key,
        );
    }

    let community_id_clone = community_id.to_string();
    let owner_key_for_db = owner_key.clone();
    db_call(pool, move |conn| {
        rekindle_db::repo::communities::delete(conn, &owner_key_for_db, &community_id_clone)
    })
    .await?;

    crate::services::audit::append(
        state,
        pool,
        &owner_key,
        rekindle_audit::AuditKind::ChannelLeft,
        serde_json::json!({ "community_id": community_id }),
    );

    tracing::info!(community = %community_id, "left community");
    Ok(())
}

pub async fn create_community_inner(
    state: &SharedState,
    pool: &Db,
    name: String,
    admission: rekindle_types::governance::AdmissionMode,
) -> Result<String, String> {
    use crate::db;
    use crate::services;

    let owner_key = state_helpers::current_owner_key(state)?;
    // The Host records the community — its row, its vault keys and its
    // store entry (`HostGovernance::insert_community`, plan E3.1 B2e).
    let community_id = services::community::create_community(state, &name, admission).await?;

    let community = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .cloned()
            .ok_or("community not found after creation")?
    };

    let creator_key = owner_key.clone();
    let creator_name = state_helpers::identity_display_name(state);
    let now = db::timestamp_now();
    let community_id_clone = community_id.clone();
    let pseudonym_key = community.my_pseudonym_key.clone().unwrap_or(creator_key);
    let roles_to_persist = community.roles.clone();
    let ok = owner_key;
    db_call(pool, move |conn| {
        let owner_role_ids = [0u32, u32::MAX];
        for role in &roles_to_persist {
            conn.execute(
                "INSERT INTO community_roles (owner_key, community_id, role_id, name, color, permissions, position, hoist, mentionable, self_assignable, exclusion_group) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    ok, community_id_clone, role.id, role.name, role.color,
                    role.permissions.cast_signed(), role.position, i32::from(role.hoist), i32::from(role.mentionable),
                    i32::from(role.self_assignable), role.exclusion_group,
                ],
            )?;
        }

        for channel in &community.channels {
            rekindle_db::repo::channels::insert(conn, &ok, &community_id_clone, &channel.row())?;
        }

        rekindle_db::repo::members::insert_if_absent(
            conn,
            &ok,
            &community_id_clone,
            &pseudonym_key,
            &creator_name,
            &owner_role_ids,
            now,
        )
    })
    .await?;

    Ok(community_id)
}

pub async fn join_community_inner(
    state: &SharedState,
    pool: &Db,
    keystore_handle: &VaultHandle,
    link: &rekindle_types::invite::InviteLink,
) -> Result<(), String> {
    use crate::db;
    use crate::services;

    let owner_key = state_helpers::current_owner_key(state)?;
    let merge_input = services::community::join_community(state, link).await?;
    let community_id = link.governance_key.as_str().to_owned();

    let (
        name,
        my_pseudonym_key,
        mek_generation,
        channels,
        my_role_ids,
        roles_to_persist,
        member_registry_key,
        slot_seed,
        my_subkey_index,
        governance_key_db,
    ) = {
        let communities = state.communities.read();
        let community = communities
            .get(&community_id)
            .ok_or("community state not found after join")?;
        (
            community.name.clone(),
            community.my_pseudonym_key.clone(),
            community.mek_generation,
            community.channels.clone(),
            community.my_role_ids.clone(),
            community.roles.clone(),
            community.member_registry_key.clone(),
            community.slot_seed.clone(),
            community.my_subkey_index,
            community.governance_key.clone(),
        )
    };

    let pseudonym_key = my_pseudonym_key.unwrap_or_else(|| owner_key.clone());
    let joiner_name = state_helpers::identity_display_name(state);

    if let Some(mek) = state_helpers::current_mek(state, &community_id, KeyScope::Community) {
        let ks = keystore_handle.lock();
        let keystore = ks.as_ref().ok_or_else(|| {
            "Stronghold keystore not open — MEK persist requires unlocked vault. \
             Try logging out and back in."
                .to_string()
        })?;
        crate::keystore::persist_mek(keystore, &community_id, KeyScope::Community, &mek)?;
    }

    if let Some(ref seed) = slot_seed {
        let ks = keystore_handle.lock();
        if let Some(ref keystore) = *ks {
            crate::keystore::persist_slot_seed(keystore, &community_id, seed);
        }
    }

    let now = db::timestamp_now();
    let community_id_clone = community_id.clone();
    let ok = owner_key.clone();
    let pk = pseudonym_key.clone();
    let mg = mek_generation.cast_signed();
    let name_for_db = name.clone();
    let channels_for_db = channels.clone();
    db_call(pool, move |conn| {
        rekindle_db::repo::communities::insert_if_absent(
            conn,
            &ok,
            &rekindle_db::repo::communities::NewCommunity {
                id: &community_id_clone,
                name: &name_for_db,
                my_role_ids: &my_role_ids,
                joined_at: now,
                dht_owner_keypair: None,
                my_pseudonym_key: &pk,
                mek_generation: mg,
                member_registry_key: member_registry_key.as_deref(),
                my_subkey_index,
                governance_key: governance_key_db.as_deref(),
            },
        )?;
        // The join's merge input into the warm cache (plan E3.1 B2), after
        // the community's row, which the cache row references.
        rekindle_db::repo::governance_cache::save(conn, &ok, &community_id_clone, &merge_input, now)?;
        rekindle_db::repo::members::insert_if_absent(
            conn,
            &ok,
            &community_id_clone,
            &pk,
            &joiner_name,
            &my_role_ids,
            now,
        )?;

        for channel in &channels_for_db {
            rekindle_db::repo::channels::insert_if_absent(conn, &ok, &community_id_clone, &channel.row())?;
        }

        for role in &roles_to_persist {
            conn.execute(
                "INSERT OR IGNORE INTO community_roles (owner_key, community_id, role_id, name, color, permissions, position, hoist, mentionable, self_assignable, exclusion_group) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    ok, community_id_clone, role.id, role.name, role.color,
                    role.permissions.cast_signed(), role.position, i32::from(role.hoist), i32::from(role.mentionable),
                    i32::from(role.self_assignable), role.exclusion_group,
                ],
            )?;
        }

        Ok(())
    })
    .await?;

    crate::services::audit::append(
        state,
        pool,
        &owner_key,
        rekindle_audit::AuditKind::ChannelJoined,
        serde_json::json!({
            "community_id": community_id,
            "community_name": name,
            "channel_count": channels.len(),
        }),
    );

    Ok(())
}
