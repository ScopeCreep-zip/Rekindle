use std::sync::Arc;

use crate::state::AppState;
use tauri::Manager;

pub(crate) fn handle_admin_keypair_grant(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    wrapped_owner_keypair: &[u8],
    wrapped_slot_seed: &[u8],
) {
    use rekindle_crypto::group::mek_distribution::unwrap_mek;
    use rekindle_crypto::group::pseudonym::derive_community_pseudonym;

    let Some(secret) = state.identity_secret.lock().as_ref().copied() else {
        tracing::warn!("no identity secret — cannot unwrap admin keypair grant");
        return;
    };
    let my_signing_key = derive_community_pseudonym(&secret, community_id);

    let Ok(sender_bytes) = hex::decode(sender_pseudonym) else {
        tracing::warn!("invalid sender pseudonym hex in admin keypair grant");
        return;
    };
    let Ok(sender_pub): Result<[u8; 32], _> = sender_bytes.try_into() else {
        tracing::warn!("sender pseudonym wrong length in admin keypair grant");
        return;
    };

    let owner_kp_bytes = match unwrap_mek(&my_signing_key, &sender_pub, wrapped_owner_keypair) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, "failed to unwrap community owner keypair");
            return;
        }
    };
    let owner_kp_str = String::from_utf8_lossy(&owner_kp_bytes).to_string();

    let slot_seed_bytes = match unwrap_mek(&my_signing_key, &sender_pub, wrapped_slot_seed) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, "failed to unwrap slot seed");
            return;
        }
    };

    {
        let mut communities = state.communities.write();
        if let Some(c) = communities.get_mut(community_id) {
            c.dht_owner_keypair = Some(owner_kp_str.clone());
            c.slot_seed = Some(hex::encode(&slot_seed_bytes));
        }
    }

    let seed_hex = hex::encode(&slot_seed_bytes);
    let ks_handle: tauri::State<'_, crate::keystore::KeystoreHandle> = app_handle.state();
    let ks = ks_handle.lock();
    if let Some(ref keystore) = *ks {
        crate::keystore::persist_slot_seed(keystore, community_id, &seed_hex);
    }
    {
        let pool: tauri::State<'_, crate::db::DbPool> = app_handle.state();
        let cid = community_id.to_string();
        let owner_key = crate::state_helpers::current_owner_key(state).unwrap_or_default();
        let owner_keypair = owner_kp_str.clone();
        crate::db_helpers::db_fire(
            pool.inner(),
            "persist community owner keypair",
            move |conn| {
                conn.execute(
                    "UPDATE communities SET dht_owner_keypair = ?1 WHERE owner_key = ?2 AND id = ?3",
                    rusqlite::params![owner_keypair, owner_key, cid],
                )?;
                Ok(())
            },
        );
    }

    tracing::info!(community = %community_id, "community owner keypair grant accepted and persisted");
}

pub(crate) fn handle_slot_keypair_grant(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    slot_index: u32,
    segment_index: u32,
    wrapped_slot_keypair: &[u8],
) {
    use rekindle_crypto::group::mek_distribution::unwrap_mek;
    use rekindle_crypto::group::pseudonym::derive_community_pseudonym;

    let Some(secret) = state.identity_secret.lock().as_ref().copied() else {
        tracing::warn!("no identity secret — cannot unwrap slot keypair grant");
        return;
    };
    let my_signing_key = derive_community_pseudonym(&secret, community_id);

    let Ok(sender_bytes) = hex::decode(sender_pseudonym) else {
        tracing::warn!("invalid sender pseudonym hex in slot keypair grant");
        return;
    };
    let Ok(sender_pub): Result<[u8; 32], _> = sender_bytes.try_into() else {
        tracing::warn!("sender pseudonym wrong length in slot keypair grant");
        return;
    };

    let slot_kp_bytes = match unwrap_mek(&my_signing_key, &sender_pub, wrapped_slot_keypair) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, "failed to unwrap slot keypair");
            return;
        }
    };
    let slot_kp_str = String::from_utf8_lossy(&slot_kp_bytes).to_string();

    {
        let mut communities = state.communities.write();
        if let Some(c) = communities.get_mut(community_id) {
            c.slot_keypair = Some(slot_kp_str.clone());
            c.my_subkey_index = Some(slot_index);
        }
    }

    let ks_handle: tauri::State<'_, crate::keystore::KeystoreHandle> = app_handle.state();
    let ks = ks_handle.lock();
    if let Some(ref keystore) = *ks {
        crate::keystore::persist_slot_keypair(keystore, community_id, &slot_kp_str);
    }

    {
        let pool: tauri::State<'_, crate::db::DbPool> = app_handle.state();
        let cid = community_id.to_string();
        let owner_key = crate::state_helpers::current_owner_key(state).unwrap_or_default();
        let idx = i64::from(slot_index);
        crate::db_helpers::db_fire(pool.inner(), "persist my_subkey_index", move |conn| {
            conn.execute(
                "UPDATE communities SET my_subkey_index = ?1 WHERE owner_key = ?2 AND id = ?3",
                rusqlite::params![idx, owner_key, cid],
            )?;
            Ok(())
        });
    }

    tracing::info!(
        community = %community_id,
        slot_index, segment_index,
        "slot keypair grant accepted and persisted"
    );

    let state_for_poll = state.clone();
    let cid_for_poll = community_id.to_string();
    tokio::spawn(async move {
        if let Err(e) =
            crate::services::community::presence_poll_tick_public(&state_for_poll, &cid_for_poll)
                .await
        {
            tracing::debug!(error = %e, "immediate presence poll after SlotKeypairGrant failed");
        }
    });
}
