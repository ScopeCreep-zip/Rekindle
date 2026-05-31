//! Phase 23.C — split from friend_runtime.rs. rotate_profile_key orchestration.

use std::sync::Arc;

use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::services;
use crate::state::AppState;
use crate::state_helpers;

pub async fn rotate_profile_key(state: &Arc<AppState>, pool: &DbPool) -> Result<(), String> {
    // Create a new profile DHT record with a fresh keypair.
    // Clone the routing_context out before .await (parking_lot guards are !Send).
    let routing_context = {
        let node = state.node.read();
        let nh = node.as_ref().ok_or("node not initialized")?;
        nh.routing_context.clone()
    };
    let temp_mgr = rekindle_protocol::dht::DHTManager::new(routing_context.clone());
    let (new_key, new_keypair) = temp_mgr
        .create_record(8)
        .await
        .map_err(|e| format!("create new profile record: {e}"))?;

    // Copy current profile data to the new record
    let (old_key_str, display_name, status_bytes, route_blob) = {
        let node = state.node.read();
        let nh = node.as_ref().ok_or("node not initialized")?;
        let ok = nh.profile_dht_key.clone().unwrap_or_default();
        let identity = state.identity.read();
        let id = identity.as_ref().ok_or("identity not set")?;
        let dn = id.display_name.clone();
        let status = id.status as u8;
        let rb = nh.route_blob.clone().unwrap_or_default();
        (ok, dn, vec![status], rb)
    };

    // Read prekey from signal manager
    let prekey_bytes = {
        let signal = state.signal_manager.read();
        if let Some(handle) = signal.as_ref() {
            match handle.manager.generate_prekey_bundle(1, Some(1), Some(1)) {
                Ok(bundle) => serde_json::to_vec(&bundle).unwrap_or_default(),
                Err(_) => Vec::new(),
            }
        } else {
            Vec::new()
        }
    };

    let record_key: veilid_core::RecordKey = new_key
        .parse()
        .map_err(|e| format!("invalid new profile key: {e}"))?;

    // Write profile subkeys to new record
    // Subkey 0: display name, 1: status, 5: prekey, 6: route blob
    let _ = routing_context
        .set_dht_value(record_key.clone(), 0, display_name.into_bytes(), None)
        .await;
    let _ = routing_context
        .set_dht_value(record_key.clone(), 1, status_bytes, None)
        .await;
    let _ = routing_context
        .set_dht_value(record_key.clone(), 5, prekey_bytes, None)
        .await;
    let _ = routing_context
        .set_dht_value(record_key.clone(), 6, route_blob, None)
        .await;

    // Update NodeHandle
    {
        let mut node = state.node.write();
        if let Some(nh) = node.as_mut() {
            nh.profile_dht_key = Some(new_key.clone());
            nh.profile_owner_keypair.clone_from(&new_keypair);
        }
    }

    // Update SQLite (both dht_record_key and dht_owner_keypair)
    let nk = new_key.clone();
    let keypair_str = new_keypair.as_ref().map(std::string::ToString::to_string);
    let owner_key = state_helpers::owner_key_or_default(state);
    db_call(pool, move |conn| {
        conn.execute(
            "UPDATE identity SET dht_record_key = ?1, dht_owner_keypair = COALESCE(?3, dht_owner_keypair) WHERE public_key = ?2",
            rusqlite::params![nk, owner_key, keypair_str],
        )?;
        Ok(())
    })
    .await?;

    // Notify all remaining friends about the new profile key
    let friend_keys: Vec<String> = {
        let friends = state.friends.read();
        friends.keys().cloned().collect()
    };
    let payload = rekindle_protocol::messaging::envelope::MessagePayload::ProfileKeyRotated {
        new_profile_dht_key: new_key.clone(),
    };
    for fk in &friend_keys {
        if let Err(e) = services::message_service::send_to_peer_raw(state, pool, fk, &payload).await
        {
            tracing::warn!(to = %fk, error = %e, "failed to send ProfileKeyRotated");
        }
    }

    tracing::info!(
        old_key = %old_key_str,
        new_key = %new_key,
        "profile DHT key rotated — {} friends notified",
        friend_keys.len()
    );

    // Phase 4 — audit entry for the rotation. The plan's `IdentityRotated`
    // variant maps to "profile DHT key rotation" — the security-relevant
    // change a user makes to break linkability with a prior key. Note we
    // do NOT log the new key itself (only that rotation happened); the
    // new key is in DHT subkey 8 anyway, but the audit trail is meant to
    // record actions, not credentials.
    let owner = state_helpers::owner_key_or_default(state);
    crate::audit_repo::append_async(
        state,
        pool,
        &owner,
        rekindle_audit::AuditKind::IdentityRotated,
        serde_json::json!({
            "friend_notify_count": friend_keys.len(),
        }),
    )
    .await;
    Ok(())
}
