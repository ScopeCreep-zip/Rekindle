//! Phase 23.C — split from friend_runtime.rs. rotate_profile_key orchestration.

use std::sync::Arc;

use crate::db_helpers::db_call;
use crate::services;
use crate::state::AppState;
use crate::state_helpers;
use rekindle_db::Db;

pub async fn rotate_profile_key(state: &Arc<AppState>, pool: &Db) -> Result<(), String> {
    let (old_key_str, old_lease) = {
        let node = state.node.read();
        let nh = node.as_ref().ok_or("node not initialized")?;
        (
            nh.profile_dht_key.clone().unwrap_or_default(),
            nh.profile_lease,
        )
    };
    let route_blob = state_helpers::our_route_blob(state).unwrap_or_default();
    let (display_name, status_message) = {
        let identity = state.identity.read();
        let id = identity.as_ref().ok_or("identity not set")?;
        (id.display_name.clone(), id.status_message.clone())
    };

    // The new record carries the same long-lived bundle as the old one.
    let prekey_bytes = {
        let signal = state.signal_manager.read();
        let handle = signal.as_ref().ok_or("signal manager not initialized")?;
        let bundle = handle
            .manager
            .current_bundle()
            .map_err(|e| format!("prekey bundle: {e}"))?;
        serde_json::to_vec(&bundle).map_err(|e| format!("serialize prekey bundle: {e}"))?
    };

    // A new profile record with a fresh owner key, written as login writes
    // one (plan C7.4: one profile creator).
    let record_pool = state_helpers::record_pool(state)?;
    let (new_lease, new_key, new_keypair, outcome) =
        rekindle_protocol::dht::profile::create_profile(
            &record_pool,
            rekindle_protocol::dht::profile::ProfileFields {
                display_name: &display_name,
                status_message: &status_message,
                prekey_bundle: &prekey_bytes,
                route_blob: &route_blob,
            },
        )
        .await
        .map_err(|e| format!("create new profile record: {e}"))?;
    if outcome.missed() {
        tracing::warn!(?outcome, "rotated profile not stored at consensus");
    }

    state_helpers::store_dht_record(
        state,
        &new_key,
        &state_helpers::DhtRecordType::Profile(new_keypair.clone(), new_lease),
    );
    // The old profile is no longer ours to write.
    if let Some(lease) = old_lease {
        record_pool.release(lease).await;
    }
    // The publisher writes the status to the new profile (it reads the
    // profile key at write time).
    services::presence_service::request_status_publish(state);

    // Update SQLite (both dht_record_key and dht_owner_keypair)
    let nk = new_key.clone();
    let keypair_str = Some(new_keypair.to_string());
    let owner_key = state_helpers::owner_key_or_default(state);
    db_call(pool, move |conn| {
        rekindle_db::repo::identity::set_owned_record(
            conn,
            &owner_key,
            rekindle_db::repo::identity::OwnedRecord::Profile,
            &nk,
            keypair_str.as_deref(),
        )
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
        if let Err(e) = services::message_service::send_to_peer(state, pool, fk, &payload).await {
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
