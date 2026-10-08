use std::sync::Arc;

use tauri::AppHandle;

use crate::services::presence_service;
use crate::state::AppState;

pub async fn handle_value_change(
    app_handle: &AppHandle,
    state: &Arc<AppState>,
    change: veilid_core::VeilidValueChange,
) {
    let key = change.key.to_string();

    // Phase 7 — watch-tier trigger. If the ValueChange is on the
    // local user's mailbox key (where peers write friend requests),
    // wake the friendship coordinator so it scans within ~500 ms
    // instead of waiting for the 30 s poll backstop. The helper
    // honours the watch trigger's dev-disable deadline (kill-switch)
    // and is a no-op when no coordinator is running.
    let own_mailbox_key = state
        .node
        .read()
        .as_ref()
        .and_then(|nh| nh.mailbox_dht_key.clone());
    if own_mailbox_key.as_deref() == Some(key.as_str()) {
        state.friendship_handle.fire_watch_trigger();
    }

    // The pool is the one owner of watch death (plan C7.8): it re-arms the
    // watch of a record it holds, which stays in the watched set meanwhile
    // (so the inspect tick does not take it over). A record it does not
    // hold was released (a left community, a removed friend), so its watch
    // ends by design.
    let held = state
        .record_pool
        .read()
        .clone()
        .is_some_and(|pool| pool.on_value_change(&change).held);

    if change.subkeys.is_empty() {
        if !held {
            crate::services::community::mark_watch_inactive(state, &key);
        }
        return;
    }

    if change.count == 0 && !held {
        crate::services::community::mark_watch_inactive(state, &key);
    }

    let subkeys: Vec<u32> = change.subkeys.iter().collect();
    let first_subkey = subkeys.first().copied();
    let inline_value = change.value.as_ref().map(|v| v.data().to_vec());
    let Ok(pool) = state.db.current() else {
        tracing::debug!("DHT value change: no identity database — dropped");
        return;
    };
    tracing::debug!(
        key = %key,
        subkeys = ?subkeys,
        has_inline = inline_value.is_some(),
        "DHT value changed"
    );

    // Changed subkeys are re-read through the session's record pool, whose
    // borrow opens the record: the plain 1-hop context read records that
    // were never opened (V19).
    let record_pool = crate::state_helpers::record_pool(state).ok();

    if crate::services::sync_communities::handle_community_record_change(state, &pool, &key).await {
        tracing::debug!(key = %key, "handled community DHT change via sync service");
        return;
    }

    // Personal cross-device sync record (architecture §28.4).
    if crate::services::cross_device_sync::watch::try_handle_personal_sync_change(
        state,
        &pool,
        &key,
        &subkeys,
        inline_value.as_deref(),
    )
    .await
    {
        return;
    }

    // DM SMPL records: try the DM dispatcher first. Returns true when the
    // key matches a row in `dms`. (Architecture §27 — DMs reuse the SMPL
    // schema universally; the watch goes through the same plumbing as
    // community records.)
    if try_handle_dm_change(state, &pool, &key, &subkeys, record_pool.as_deref()).await {
        return;
    }

    for &subkey in &subkeys {
        let use_inline = Some(subkey) == first_subkey;
        let value = if use_inline && inline_value.is_some() {
            inline_value.clone().unwrap_or_default()
        } else if let Some(ref record_pool) = record_pool {
            match record_pool.read_once(&change.key, subkey, true).await {
                Ok(Some(v)) => v.data().to_vec(),
                Ok(None) => {
                    tracing::debug!(subkey, key = %key, "subkey has no value");
                    continue;
                }
                Err(e) => {
                    tracing::warn!(subkey, key = %key, error = %e, "failed to fetch subkey");
                    continue;
                }
            }
        } else {
            tracing::debug!(subkey, "no routing context to fetch subkey value");
            continue;
        };
        presence_service::handle_value_change(app_handle, state, &key, &[subkey], &value);
        // Mutual Aid (architecture §14.3): we hold a watch slot for this
        // record, so gossip-relay the notification to community peers
        // who may not — the receivers fetch the new value themselves
        // via `get_dht_value` (no ciphertext crosses gossip).
        relay_watch_change(state, &key, subkey, &value);
    }
}

async fn try_handle_dm_change(
    state: &Arc<AppState>,
    pool: &rekindle_db::Db,
    record_key: &str,
    subkeys: &[u32],
    record_pool: Option<&rekindle_protocol::dht::pool::RecordPool>,
) -> bool {
    use crate::db_helpers::db_call_or_default;
    use crate::state_helpers;

    let owner_key = state_helpers::owner_key_or_default(state);
    if owner_key.is_empty() {
        return false;
    }
    let owner = owner_key;
    let record = record_key.to_string();
    let exists: bool = db_call_or_default(pool, move |conn| {
        Ok(conn
            .query_row(
                "SELECT 1 FROM dms WHERE owner_key = ?1 AND record_key = ?2 LIMIT 1",
                rusqlite::params![owner, record],
                |_| Ok(()),
            )
            .is_ok())
    })
    .await;
    if !exists {
        return false;
    }

    let Some(record_pool) = record_pool else {
        return true;
    };
    let Ok(parsed) = record_key.parse::<veilid_core::RecordKey>() else {
        return true;
    };
    for &subkey in subkeys {
        if let Ok(Some(value)) = record_pool.read_once(&parsed, subkey, true).await {
            if let Err(e) = crate::services::dm::handle_dm_subkey_change(
                state,
                pool,
                record_key,
                subkey,
                value.data(),
            )
            .await
            {
                tracing::debug!(record_key, subkey, error = %e, "dm subkey handler dropped");
            }
        }
    }
    true
}

fn relay_watch_change(state: &Arc<AppState>, record_key: &str, subkey: u32, value: &[u8]) {
    use rekindle_codec::community::envelope::CommunityEnvelope;

    let community_id_and_pseudonym = {
        let communities = state.communities.read();
        communities.values().find_map(|cs| {
            let matched = cs.governance_key.as_deref() == Some(record_key)
                || cs.member_registry_key.as_deref() == Some(record_key)
                || cs.channel_log_keys.values().any(|k| k == record_key)
                || cs.governance_state.as_ref().is_some_and(|gov| {
                    gov.segments
                        .iter()
                        .any(|s| s.governance_key == record_key || s.registry_key == record_key)
                        || gov
                            .channel_segment_records
                            .values()
                            .any(|csr| csr.record_key == record_key)
                });
            matched.then(|| {
                (
                    cs.id.clone(),
                    cs.my_pseudonym_key.clone().unwrap_or_default(),
                )
            })
        })
    };
    let Some((community_id, observer)) = community_id_and_pseudonym else {
        return;
    };
    if observer.is_empty() {
        return;
    }
    let envelope = CommunityEnvelope::WatchRelay {
        record_key: record_key.to_string(),
        subkey,
        content_hash: blake3::hash(value).to_hex().to_string(),
        observer_pseudonym: observer,
    };
    if let Err(e) =
        crate::services::community::gossip::send_to_mesh(state, &community_id, &envelope)
    {
        tracing::debug!(community = %community_id, error = %e, "watch relay gossip failed");
    }
}
