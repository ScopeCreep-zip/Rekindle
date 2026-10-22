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
