use std::sync::Arc;

use crate::db::DbPool;
use crate::state::AppState;
use crate::state_helpers;
use tauri::{Emitter, Manager};

pub(crate) struct JoinAcceptedData<'a> {
    mek_wire_bytes: &'a [u8],
    mek_generation: u64,
    members: &'a [rekindle_protocol::dht::community::types::MemberSummary],
    member_registry_key: Option<&'a str>,
    slot_index: Option<u32>,
    wrapped_slot_seed: Option<&'a [u8]>,
}

pub(crate) fn join_accepted_data<'a>(
    mek_wire_bytes: &'a [u8],
    mek_generation: u64,
    members: &'a [rekindle_protocol::dht::community::types::MemberSummary],
    member_registry_key: Option<&'a str>,
    slot_index: Option<u32>,
    wrapped_slot_seed: Option<&'a [u8]>,
) -> JoinAcceptedData<'a> {
    JoinAcceptedData {
        mek_wire_bytes,
        mek_generation,
        members,
        member_registry_key,
        slot_index,
        wrapped_slot_seed,
    }
}

pub(crate) enum MekDecryptResult {
    Decrypted(String),
    NeedRefresh,
    Failed,
}

pub(crate) fn decrypt_with_cached_mek(
    mek_cache: &std::collections::HashMap<
        String,
        rekindle_crypto::group::media_key::MediaEncryptionKey,
    >,
    community_id: &str,
    ciphertext: &[u8],
    mek_generation: u64,
) -> MekDecryptResult {
    match mek_cache.get(community_id) {
        Some(mek) if mek.generation() == mek_generation => match mek.decrypt(ciphertext) {
            Ok(plaintext) => {
                MekDecryptResult::Decrypted(String::from_utf8(plaintext).unwrap_or_default())
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to decrypt community message");
                MekDecryptResult::Failed
            }
        },
        Some(mek) => {
            tracing::warn!(
                have = mek.generation(),
                need = mek_generation,
                "MEK generation mismatch — fetching updated MEK from DHT vault"
            );
            MekDecryptResult::NeedRefresh
        }
        None => {
            tracing::warn!(community = %community_id, "no MEK cached for community");
            MekDecryptResult::Failed
        }
    }
}

pub(crate) fn fetch_mek_from_dht(
    _app_handle: &tauri::AppHandle,
    _state: &Arc<AppState>,
    community_id: &str,
) {
    tracing::debug!(
        community = %community_id,
        "fetch_mek_from_dht: v2.0 uses invite-time MEK distribution — vault read skipped"
    );
}

fn handle_slot_seed_grant(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    slot_index: u32,
    wrapped_slot_seed: &[u8],
) {
    use rekindle_crypto::group::mek_distribution::unwrap_mek;
    use rekindle_crypto::group::pseudonym::derive_community_pseudonym;

    let Some(secret) = state.identity_secret.lock().as_ref().copied() else {
        tracing::warn!("no identity secret — cannot unwrap slot seed");
        return;
    };
    let my_signing_key = derive_community_pseudonym(&secret, community_id);

    let Ok(sender_bytes) = hex::decode(sender_pseudonym) else {
        tracing::warn!("invalid sender pseudonym hex in slot seed grant");
        return;
    };
    let Ok(sender_pub): Result<[u8; 32], _> = sender_bytes.try_into() else {
        tracing::warn!("sender pseudonym wrong length in slot seed grant");
        return;
    };

    let seed_bytes = match unwrap_mek(&my_signing_key, &sender_pub, wrapped_slot_seed) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, "failed to unwrap slot seed");
            return;
        }
    };
    let seed_hex = String::from_utf8_lossy(&seed_bytes).to_string();

    let seed_raw = match hex::decode(&seed_hex) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, "slot seed is not valid hex");
            return;
        }
    };
    let Ok(seed_array): Result<[u8; 32], _> = seed_raw.try_into() else {
        tracing::warn!("slot seed wrong length (expected 32 bytes)");
        return;
    };
    let slot_kp = match rekindle_secrets::derive::derive_slot_keypair(&seed_array, slot_index) {
        Ok(sk) => crate::services::community::create::slot_signing_to_veilid(&sk),
        Err(e) => {
            tracing::warn!(error = %e, slot_index, "failed to derive slot keypair from seed");
            return;
        }
    };
    let slot_kp_str = slot_kp.to_string();

    {
        let mut communities = state.communities.write();
        if let Some(c) = communities.get_mut(community_id) {
            c.slot_seed = Some(seed_hex.clone());
            c.slot_keypair = Some(slot_kp_str.clone());
            c.my_subkey_index = Some(slot_index);
        }
    }

    let ks_handle: tauri::State<'_, crate::keystore::KeystoreHandle> = app_handle.state();
    let ks = ks_handle.lock();
    if let Some(ref keystore) = *ks {
        crate::keystore::persist_slot_seed(keystore, community_id, &seed_hex);
        crate::keystore::persist_slot_keypair(keystore, community_id, &slot_kp_str);
    }

    {
        let pool: tauri::State<'_, crate::db::DbPool> = app_handle.state();
        let cid = community_id.to_string();
        let owner_key = crate::state_helpers::current_owner_key(state).unwrap_or_default();
        let idx = i64::from(slot_index);
        crate::db_helpers::db_fire(
            pool.inner(),
            "persist my_subkey_index from seed",
            move |conn| {
                conn.execute(
                    "UPDATE communities SET my_subkey_index = ?1 WHERE owner_key = ?2 AND id = ?3",
                    rusqlite::params![idx, owner_key, cid],
                )?;
                Ok(())
            },
        );
    }

    tracing::info!(
        community = %community_id,
        slot_index,
        "slot seed received — derived slot keypair locally"
    );

    let state_for_poll = state.clone();
    let cid_for_poll = community_id.to_string();
    tokio::spawn(async move {
        if let Err(e) =
            crate::services::community::presence_poll_tick_public(&state_for_poll, &cid_for_poll)
                .await
        {
            tracing::debug!(error = %e, "immediate presence poll after slot seed grant failed");
        }
    });
}

pub(crate) async fn handle_join_accepted(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    data: JoinAcceptedData<'_>,
) {
    use crate::channels::CommunityEvent;
    use rekindle_crypto::group::media_key::MediaEncryptionKey;
    use rekindle_protocol::dht::community::types::MemberSummary;

    let my_pseudonym = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .and_then(|cs| cs.my_pseudonym_key.clone())
    };
    if my_pseudonym.as_deref() == Some(sender_pseudonym) {
        tracing::debug!(community = %community_id, "ignoring self-JoinAccepted loopback");
        return;
    }

    if !data.mek_wire_bytes.is_empty() {
        if let Some(mek) = MediaEncryptionKey::from_wire_bytes(data.mek_wire_bytes) {
            let gen = mek.generation();
            state.mek_cache.lock().insert(community_id.to_string(), mek);
            tracing::info!(
                community = %community_id,
                mek_generation = gen,
                "cached MEK from JoinAccepted"
            );
        } else {
            tracing::warn!(community = %community_id, "JoinAccepted contained invalid MEK wire bytes");
        }
    }

    let parsed_members: Vec<MemberSummary> = data.members.to_vec();

    if let Some(ref my_pk) = my_pseudonym {
        if let Some(me) = parsed_members.iter().find(|m| m.pseudonym_key == *my_pk) {
            let mut communities = state.communities.write();
            if let Some(cs) = communities.get_mut(community_id) {
                if cs.my_subkey_index.is_none() {
                    cs.my_subkey_index = Some(me.subkey_index);
                    tracing::info!(
                        community = %community_id,
                        subkey_index = me.subkey_index,
                        "extracted my_subkey_index from members list (backup path)"
                    );
                }
            }
        }
    }

    if data.slot_index.is_none() {
        if let Some(ref my_pk) = my_pseudonym {
            if let Some(me) = parsed_members.iter().find(|m| m.pseudonym_key == *my_pk) {
                let pool: tauri::State<'_, DbPool> = app_handle.state();
                let owner_key = state_helpers::current_owner_key(state).unwrap_or_default();
                let cid = community_id.to_string();
                let idx = i64::from(me.subkey_index);
                crate::db_helpers::db_fire(
                    pool.inner(),
                    "backup my_subkey_index from members list",
                    move |conn| {
                        conn.execute(
                            "UPDATE communities SET my_subkey_index = ?1 WHERE owner_key = ?2 AND id = ?3",
                            rusqlite::params![idx, owner_key, cid],
                        )?;
                        Ok(())
                    },
                );
            }
        }
    }

    {
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(community_id) {
            cs.mek_generation = data.mek_generation;
            if cs.member_registry_key.is_none() {
                if let Some(rk) = data.member_registry_key {
                    cs.member_registry_key = Some(rk.to_string());
                }
            } else if let Some(rk) = data.member_registry_key {
                cs.member_registry_key = Some(rk.to_string());
            }
        }
    }

    {
        let pool: tauri::State<'_, DbPool> = app_handle.state();
        let owner_key = state_helpers::current_owner_key(state).unwrap_or_default();
        let cid = community_id.to_string();
        let rk_str = data.member_registry_key.map(str::to_string);
        let _ = crate::db_helpers::db_call(pool.inner(), move |conn| {
            if let Some(ref rk) = rk_str {
                conn.execute(
                    "UPDATE communities SET member_registry_key = ?1 WHERE owner_key = ?2 AND id = ?3",
                    rusqlite::params![rk, owner_key, cid],
                )?;
            }
            Ok(())
        })
        .await;
    }

    if !parsed_members.is_empty() {
        let pool: tauri::State<'_, DbPool> = app_handle.state();
        let owner_key = state_helpers::current_owner_key(state).unwrap_or_default();
        let cid = community_id.to_string();
        let members_for_db = parsed_members.clone();
        let result = crate::db_helpers::db_call(pool.inner(), move |conn| {
            for m in &members_for_db {
                let role_ids_json =
                    serde_json::to_string(&m.role_ids).unwrap_or_else(|_| "[0,1]".into());
                conn.execute(
                    "INSERT OR REPLACE INTO community_members \
                     (owner_key, community_id, pseudonym_key, display_name, role_ids, joined_at, subkey_index, onboarding_complete, timeout_until) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    rusqlite::params![
                        owner_key,
                        cid,
                        m.pseudonym_key,
                        m.display_name,
                        role_ids_json,
                        m.joined_at.cast_signed(),
                        m.subkey_index,
                        i32::from(m.onboarding_complete),
                        m.timeout_until.map(u64::cast_signed),
                    ],
                )?;
            }
            Ok(())
        })
        .await;
        if let Err(e) = result {
            tracing::warn!(community = %community_id, error = %e, "failed to persist JoinAccepted members to SQLite");
        }
    }

    if !parsed_members.is_empty() {
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(community_id) {
            for m in &parsed_members {
                cs.known_members.insert(m.pseudonym_key.clone());
            }
        }
    }

    if !data.mek_wire_bytes.is_empty() {
        let ks_handle: tauri::State<'_, crate::keystore::KeystoreHandle> = app_handle.state();
        let ks = ks_handle.lock();
        if let Some(ref keystore) = *ks {
            let mek_cache = state.mek_cache.lock();
            if let Some(mek) = mek_cache.get(community_id) {
                crate::keystore::persist_mek(keystore, community_id, mek);
                tracing::debug!(community = %community_id, "persisted MEK to Stronghold after JoinAccepted");
            }
        }
    }

    if let (Some(idx), Some(wrapped_seed)) = (data.slot_index, data.wrapped_slot_seed) {
        handle_slot_seed_grant(
            app_handle,
            state,
            community_id,
            sender_pseudonym,
            idx,
            wrapped_seed,
        );
    }

    let _ = app_handle.emit(
        "community-event",
        CommunityEvent::JoinAccepted {
            community_id: community_id.to_string(),
        },
    );

    tracing::info!(
        community = %community_id,
        mek_generation = data.mek_generation,
        member_count = parsed_members.len(),
        has_slot_keypair = data.slot_index.is_some(),
        "JoinAccepted processed — join state updated"
    );

    spawn_peer_bootstrap(state.clone(), community_id.to_string(), parsed_members);
}

fn spawn_peer_bootstrap(
    state: Arc<AppState>,
    community_id: String,
    members: Vec<rekindle_protocol::dht::community::types::MemberSummary>,
) {
    tokio::spawn(async move {
        let (registry_key, my_pseudo) = {
            let communities = state.communities.read();
            let cs = communities.get(&community_id);
            (
                cs.and_then(|c| c.member_registry_key.clone()),
                cs.and_then(|c| c.my_pseudonym_key.clone()),
            )
        };
        let Some(rk) = registry_key else { return };
        let Some(rc) = crate::state_helpers::routing_context(&state) else {
            return;
        };

        let Ok(reg_typed_key) = rk.parse::<veilid_core::RecordKey>() else {
            return;
        };
        if let Err(e) = rc.open_dht_record(reg_typed_key.clone(), None).await {
            tracing::debug!(error = %e, "failed to open registry for peer bootstrap");
            return;
        }

        let mut found_peers = 0u32;
        for member in &members {
            if my_pseudo.as_deref() == Some(&member.pseudonym_key) {
                continue;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            if let Ok(Some(val)) = rc
                .get_dht_value(reg_typed_key.clone(), member.subkey_index, false)
                .await
            {
                if val.data().is_empty() {
                    continue;
                }
                if let Ok(presence) =
                    serde_json::from_slice::<rekindle_types::presence::MemberPresence>(val.data())
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
                    if !presence.route_blob.is_empty() && presence.status != "offline" {
                        let mut communities = state.communities.write();
                        if let Some(cs) = communities.get_mut(&community_id) {
                            if let Some(ref mut gossip) = cs.gossip {
                                let om = crate::state::OnlineMember {
                                    route_blob: presence.route_blob,
                                    status: presence.status,
                                    last_seen: rekindle_utils::timestamp_secs(),
                                };
                                gossip
                                    .online_members
                                    .insert(member.pseudonym_key.clone(), om.clone());
                                gossip.peers.insert(member.pseudonym_key.clone(), om);
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
    });
}
