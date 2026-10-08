//! Phase 23.E — `MembershipEventDeps` impl for `GovernanceAdapter`.
//!
//! These method bodies are the AppState mutation + SQLite + Stronghold +
//! Veilid DHT code lifted verbatim from the deleted
//! `services/veilid/legacy/` handlers. The pure decision logic
//! (MEK-generation matching, onboarding answer→role validation, keypair
//! unwrap/derive) lives crate-side in
//! `rekindle_governance_runtime::membership_events`; this file is the
//! Tauri/Veilid/SQLite boundary it calls through.

use std::sync::Arc;

use async_trait::async_trait;
use tauri::Manager;

use rekindle_governance_runtime::membership_events::{
    MemberUpsertRow, MembershipEventDeps, SlotGrantUpdate,
};
use rekindle_secrets::sync_key::SyncKey;

use crate::services::cross_device_sync::{
    open_personal_sync_record, read_read_state, write_read_state,
};
use crate::state::AppState;
use crate::state_helpers;
use rekindle_db::Db;

use super::GovernanceAdapter;

#[async_trait]
impl MembershipEventDeps for GovernanceAdapter {
    // ---------- Roles ----------

    fn persist_member_roles(
        &self,
        community_id: &str,
        pseudonym_hex: &str,
        role_ids: &[u32],
        is_self: bool,
    ) {
        if is_self && !role_ids.is_empty() {
            let mut communities = self.state.communities.write();
            if let Some(cs) = communities.get_mut(community_id) {
                cs.my_role_ids = role_ids.to_vec();
            }
        }
        let Ok(owner_key) = state_helpers::current_owner_key(&self.state) else {
            return;
        };
        let cid = community_id.to_string();
        let pk = pseudonym_hex.to_string();
        let rids = role_ids.to_vec();
        crate::db_helpers::db_fire(&self.pool, "member_roles_changed_persist", move |conn| {
            rekindle_db::repo::members::set_role_ids(conn, &owner_key, &cid, &pk, &rids, false)?;
            if is_self {
                rekindle_db::repo::communities::set_my_role_ids(conn, &owner_key, &cid, &rids)?;
            }
            Ok(())
        });
    }

    // ---------- JoinAccepted ----------

    async fn set_mek_generation_and_registry(
        &self,
        community_id: &str,
        mek_generation: u64,
        member_registry_key: Option<&str>,
    ) {
        {
            let mut communities = self.state.communities.write();
            if let Some(cs) = communities.get_mut(community_id) {
                cs.mek_generation = mek_generation;
                if let Some(rk) = member_registry_key {
                    cs.member_registry_key = Some(rk.to_string());
                }
            }
        }
        let owner_key = state_helpers::current_owner_key(&self.state).unwrap_or_default();
        let cid = community_id.to_string();
        let rk_str = member_registry_key.map(str::to_string);
        if let Some(rk) = rk_str {
            if let Err(e) = crate::db_helpers::db_call(&self.pool, move |conn| {
                rekindle_db::repo::communities::set(
                    conn,
                    &owner_key,
                    &cid,
                    rekindle_db::repo::communities::Column::MemberRegistryKey,
                    rk,
                )
            })
            .await
            {
                tracing::warn!(community = %community_id, error = %e, "member registry key not persisted");
            }
        }
    }

    async fn upsert_members(&self, community_id: &str, members: Vec<MemberUpsertRow>) {
        if members.is_empty() {
            return;
        }
        let owner_key = state_helpers::current_owner_key(&self.state).unwrap_or_default();
        let cid = community_id.to_string();
        let result = crate::db_helpers::db_call(&self.pool, move |conn| {
            for m in &members {
                rekindle_db::repo::members::replace_accepted(
                    conn,
                    &owner_key,
                    &cid,
                    &rekindle_db::repo::members::AcceptedMember {
                        pseudonym_key: &m.pseudonym_hex,
                        display_name: &m.display_name,
                        role_ids: &m.role_ids,
                        joined_at: m.joined_at.cast_signed(),
                        subkey_index: m.subkey_index,
                        onboarding_complete: m.onboarding_complete,
                        timeout_until: m.timeout_until.map(u64::cast_signed),
                    },
                )?;
            }
            Ok(())
        })
        .await;
        if let Err(e) = result {
            tracing::warn!(community = %community_id, error = %e, "failed to persist JoinAccepted members to SQLite");
        }
    }

    fn backfill_my_subkey_index(&self, community_id: &str, subkey_index: u32, persist: bool) {
        {
            let mut communities = self.state.communities.write();
            if let Some(cs) = communities.get_mut(community_id) {
                if cs.my_subkey_index.is_none() {
                    cs.my_subkey_index = Some(subkey_index);
                    tracing::info!(
                        community = %community_id,
                        subkey_index,
                        "extracted my_subkey_index from members list (backup path)"
                    );
                }
            }
        }
        if !persist {
            return;
        }
        let owner_key = state_helpers::current_owner_key(&self.state).unwrap_or_default();
        let cid = community_id.to_string();
        let idx = i64::from(subkey_index);
        crate::db_helpers::db_fire(
            &self.pool,
            "backup my_subkey_index from members list",
            move |conn| {
                rekindle_db::repo::communities::set(
                    conn,
                    &owner_key,
                    &cid,
                    rekindle_db::repo::communities::Column::MySubkeyIndex,
                    idx,
                )
            },
        );
    }

    fn insert_known_members(&self, community_id: &str, pseudonyms: &[String]) {
        let mut communities = self.state.communities.write();
        if let Some(cs) = communities.get_mut(community_id) {
            for pseudonym in pseudonyms {
                cs.known_members.insert(pseudonym.clone());
            }
        }
    }

    fn persist_community_mek_to_keystore(&self, community_id: &str) {
        let ks_handle: tauri::State<'_, crate::keystore::KeystoreHandle> = self.app_handle.state();
        let ks = ks_handle.lock();
        if let Some(ref keystore) = *ks {
            if let Some(mek) = crate::state_helpers::current_mek(
                &self.state,
                community_id,
                rekindle_types::channel_keys::KeyScope::Community,
            ) {
                if let Err(e) = crate::keystore::persist_mek(
                    keystore,
                    community_id,
                    rekindle_types::channel_keys::KeyScope::Community,
                    &mek,
                ) {
                    tracing::warn!(community = %community_id, error = %e, "community MEK from JoinAccepted not persisted");
                }
            }
        }
    }

    fn spawn_peer_bootstrap(&self, community_id: &str, members: Vec<MemberUpsertRow>) {
        let state = Arc::clone(&self.state);
        let community_id = community_id.to_string();
        crate::state_helpers::spawn_in_login_with_token(
            &state.clone(),
            "peer bootstrap",
            |stop| async move {
                let (registry_key, my_pseudo) = {
                    let communities = state.communities.read();
                    let cs = communities.get(&community_id);
                    (
                        cs.and_then(|c| c.member_registry_key.clone()),
                        cs.and_then(|c| c.my_pseudonym_key.clone()),
                    )
                };
                let Some(rk) = registry_key else { return };
                let Ok(pool) = state_helpers::record_pool(&state) else {
                    return;
                };
                let Ok(reg_typed_key) = rk.parse::<veilid_core::RecordKey>() else {
                    return;
                };

                let mut found_peers = 0u32;
                for member in &members {
                    if my_pseudo.as_deref() == Some(&member.pseudonym_hex) {
                        continue;
                    }
                    let pace = tokio::time::sleep(std::time::Duration::from_millis(200));
                    if stop.run_until_cancelled(pace).await.is_none() {
                        return;
                    }
                    if let Ok(Some(val)) = pool
                        .read_once(&reg_typed_key, member.subkey_index, false)
                        .await
                    {
                        if val.data().is_empty() {
                            continue;
                        }
                        if let Ok(presence) = serde_json::from_slice::<
                            rekindle_types::presence::MemberPresence,
                        >(val.data())
                        {
                            // Architecture §26 W26 — verify the presence row was
                            // signed by the claimed pseudonym before treating it
                            // as authoritative routing info.
                            let Ok(sig_arr): Result<[u8; 64], _> =
                                presence.signature.as_slice().try_into()
                            else {
                                continue;
                            };
                            if rekindle_secrets::derive::verify_pseudonym_signature(
                                &presence.pseudonym_key.0,
                                &presence.signing_bytes(),
                                &sig_arr,
                            )
                            .is_err()
                            {
                                continue;
                            }
                            // Liveness ≠ reachability. A just-joined member is
                            // online the moment it heartbeats; its route may not
                            // have been allocated yet. Populate `online_members`
                            // (roster) regardless of route, but only add to
                            // `peers` (the set we actually send bytes to) once a
                            // route is present.
                            if presence.status != "offline" {
                                let mut communities = state.communities.write();
                                if let Some(cs) = communities.get_mut(&community_id) {
                                    if let Some(ref mut gossip) = cs.gossip {
                                        let route_present = !presence.route_blob.is_empty();
                                        let om = crate::state::OnlineMember {
                                            location: presence.session.location.clone(),
                                            last_active: presence.session.last_active,
                                            route_blob: presence.route_blob,
                                            status: presence.status,
                                            last_seen: rekindle_utils::timestamp_secs(),
                                        };
                                        gossip
                                            .online_members
                                            .insert(member.pseudonym_hex.clone(), om.clone());
                                        if route_present {
                                            gossip.peers.insert(member.pseudonym_hex.clone(), om);
                                        }
                                        found_peers += 1;
                                    }
                                }
                            }
                        }
                    }
                }
                if found_peers > 0 {
                    tracing::info!(
                        community = %community_id,
                        peers = found_peers,
                        "bootstrapped gossip peers from JoinAccepted member list"
                    );
                }
            },
        );
    }

    // ---------- Grants / slot seed ----------

    fn apply_slot_grant(&self, community_id: &str, update: SlotGrantUpdate) {
        {
            let mut communities = self.state.communities.write();
            if let Some(c) = communities.get_mut(community_id) {
                if let Some(ref seed) = update.slot_seed_hex {
                    c.slot_seed = Some(seed.clone());
                }
                if let Some(ref kp) = update.slot_keypair {
                    c.slot_keypair = Some(kp.clone());
                }
                if let Some(ref owner_kp) = update.dht_owner_keypair {
                    c.dht_owner_keypair = Some(owner_kp.clone());
                }
                if let Some(idx) = update.my_subkey_index {
                    c.my_subkey_index = Some(idx);
                }
            }
        }

        {
            let ks_handle: tauri::State<'_, crate::keystore::KeystoreHandle> =
                self.app_handle.state();
            let ks = ks_handle.lock();
            if let Some(ref keystore) = *ks {
                if let Some(ref seed) = update.slot_seed_hex {
                    crate::keystore::persist_slot_seed(keystore, community_id, seed);
                }
                if let Some(ref kp) = update.slot_keypair {
                    crate::keystore::persist_slot_keypair(keystore, community_id, kp);
                }
            }
        }

        let owner_kp = update.dht_owner_keypair;
        let subkey_idx = update.my_subkey_index;
        if owner_kp.is_none() && subkey_idx.is_none() {
            return;
        }
        let owner_key = state_helpers::current_owner_key(&self.state).unwrap_or_default();
        let cid = community_id.to_string();
        crate::db_helpers::db_fire(&self.pool, "apply slot grant", move |conn| {
            use rekindle_db::repo::communities::{set, Column};
            if let Some(kp) = owner_kp {
                set(conn, &owner_key, &cid, Column::DhtOwnerKeypair, kp)?;
            }
            if let Some(idx) = subkey_idx {
                set(conn, &owner_key, &cid, Column::MySubkeyIndex, idx)?;
            }
            Ok(())
        });
    }

    fn spawn_presence_poll_tick(&self, community_id: &str) {
        let state = Arc::clone(&self.state);
        let community_id = community_id.to_string();
        crate::state_helpers::login_scope_or_closed(&state).spawn_or_drop("presence poll tick", async move {
            if let Err(e) =
                crate::services::community::presence_poll_tick_public(&state, &community_id).await
            {
                tracing::debug!(error = %e, community = %community_id, "immediate presence poll after slot grant failed");
            }
        });
    }

    // ---------- Onboarding ----------

    fn persist_onboarding_completion(
        &self,
        community_id: &str,
        pseudonym_hex: &str,
        role_ids: &[u32],
        is_self: bool,
    ) {
        {
            let mut communities = self.state.communities.write();
            if let Some(cs) = communities.get_mut(community_id) {
                cs.member_roles
                    .insert(pseudonym_hex.to_string(), role_ids.to_vec());
                if is_self {
                    cs.onboarding_complete = true;
                }
            }
        }
        let Ok(owner_key) = state_helpers::current_owner_key(&self.state) else {
            return;
        };
        let cid = community_id.to_string();
        let pk = pseudonym_hex.to_string();
        let role_ids = role_ids.to_vec();
        crate::db_helpers::db_fire(&self.pool, "persist onboarding completion", move |conn| {
            rekindle_db::repo::members::set_role_ids(conn, &owner_key, &cid, &pk, &role_ids, true)
        });
    }

    fn push_onboarding_complete_to_sync(&self, community_id: &str) {
        let state = Arc::clone(&self.state);
        let pool = self.pool.clone();
        let community_id = community_id.to_string();
        crate::state_helpers::login_scope_or_closed(&state).spawn_or_drop("onboarding sync push", async move {
            if let Err(e) =
                push_onboarding_complete_to_sync_inner(&state, &pool, &community_id).await
            {
                tracing::warn!(community = %community_id, error = %e, "failed to push onboarding completion to personal sync");
            }
        });
    }

    // ---------- Peer-assisted join ----------

    fn bump_invite_uses(&self, community_id: &str, invite_code: &str) {
        let code_hash = rekindle_secrets::invite::hash_invite_code(invite_code);
        let cid = community_id.to_string();
        let owner_key = state_helpers::current_owner_key(&self.state).unwrap_or_default();
        let app_for_emit = self.app_handle.clone();
        let cid_for_emit = cid.clone();
        let code_hash_for_emit = code_hash.clone();
        crate::db_helpers::db_fire(&self.pool, "increment invite uses counter", move |conn| {
            let updated = conn.execute(
                "UPDATE community_invites SET uses = uses + 1 \
                 WHERE owner_key = ?1 AND community_id = ?2 AND code_hash = ?3",
                rusqlite::params![&owner_key, &cid, &code_hash],
            )?;
            if updated > 0 {
                let new_use_count: i64 = conn
                    .query_row(
                        "SELECT uses FROM community_invites \
                         WHERE owner_key = ?1 AND community_id = ?2 AND code_hash = ?3",
                        rusqlite::params![&owner_key, &cid, &code_hash],
                        |row| row.get(0),
                    )
                    .unwrap_or(0);
                crate::event_dispatch::emit_subscription(
                    &app_for_emit,
                    &rekindle_types::subscription_events::SubscriptionEvent::Governance(
                        rekindle_types::subscription_events::GovernanceEvent::InviteUsed {
                            community: cid_for_emit,
                            code_hash: code_hash_for_emit,
                            uses: u32::try_from(new_use_count).unwrap_or(u32::MAX),
                        },
                    ),
                );
            }
            Ok(())
        });
    }
}

/// Architecture §28.4 — flip the `onboarding_complete[community_id]` bit
/// on the personal SMPL ReadState (subkey 1). Reads → merges → writes.
/// No-ops gracefully when the personal sync record hasn't been
/// provisioned yet (fresh install before pairing).
async fn push_onboarding_complete_to_sync_inner(
    state: &Arc<AppState>,
    pool: &Db,
    community_id: &str,
) -> Result<(), String> {
    let Some(handle) = open_personal_sync_record(state, pool).await else {
        return Ok(());
    };
    let master_secret = state
        .identity_secret
        .lock()
        .as_ref()
        .copied()
        .ok_or_else(|| "identity secret not available".to_string())?;
    let sync_key = SyncKey::from_master_secret(&master_secret);
    let mut state_doc = read_read_state(state, &handle, &sync_key)
        .await
        .unwrap_or_default();
    state_doc
        .onboarding_complete
        .insert(community_id.to_string(), true);
    write_read_state(state, &handle, &sync_key, state_doc).await?;
    Ok(())
}
