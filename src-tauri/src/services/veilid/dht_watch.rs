use std::sync::Arc;

use crate::state::AppState;

pub async fn handle_value_change(state: &Arc<AppState>, change: veilid_core::VeilidValueChange) {
    // The port's form of the change (plan C9.1): keys as text, seqs whole.
    let change = rekindle_protocol::dht::pool::value_change(&change);
    let key = change.key.clone();

    // The pool is the one owner of watch death (plan C7.8): it re-arms the
    // watch of a record it holds, which stays in the watched set meanwhile
    // (so the inspect tick does not take it over). A record it does not
    // hold was released (a left community, a removed friend), so its watch
    // ends by design.
    let held = crate::state_helpers::record_pool(state)
        .is_ok_and(|pool| pool.on_value_change(&change).held);

    if change.subkeys.is_empty() {
        if !held {
            crate::services::community::mark_watch_inactive(state, &key);
        }
        return;
    }

    if change.count == 0 && !held {
        crate::services::community::mark_watch_inactive(state, &key);
    }

    let subkeys: Vec<u32> = change.subkeys.clone();
    let first_subkey = subkeys.first().copied();
    let inline_value = change.value.as_ref().map(|v| v.data.clone());
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

    match crate::services::sync_communities::handle_community_record_change(state, &pool, &key)
        .await
    {
        Ok(true) => {
            tracing::debug!(key = %key, "handled community DHT change via sync service");
            return;
        }
        Ok(false) => {}
        Err(e) => {
            tracing::debug!(key = %key, error = %e, "DHT change while locked — not routed");
            return;
        }
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

    // An invite's inbox, a friend's participant record or a friend's
    // profile: the Host reads every changed subkey (plan E2.5).
    if let Some(host) = state.host.current() {
        if host
            .friend_presence()
            .on_watched_change(&key, &subkeys, inline_value.as_deref())
            .await
        {
            return;
        }
    }

    for &subkey in &subkeys {
        let use_inline = Some(subkey) == first_subkey;
        let value = if use_inline && inline_value.is_some() {
            inline_value.clone().unwrap_or_default()
        } else if let Some(ref record_pool) = record_pool {
            match record_pool.read_once(&key, subkey, true).await {
                Ok(Some(v)) => v.data,
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
        // Mutual Aid (architecture §14.3): we hold a watch slot for this
        // record, so gossip-relay the notification to community peers
        // who may not — the receivers fetch the new value themselves
        // via `get_dht_value` (no ciphertext crosses gossip).
        relay_watch_change(state, &key, subkey, &value);
    }
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
