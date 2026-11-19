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

    match crate::services::sync_communities::handle_community_record_change(state, &key, &subkeys)
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

    // The carried value relays at once; then every hinted subkey, that one
    // too, is read from the network, which clears Veilid's report mark so
    // its next change is reported even in a coalesced push (the
    // `ValueChange` contract, plan VU3).
    let inline = first_subkey.zip(inline_value);
    if let Some((subkey, value)) = &inline {
        relay_watch_change(state, &key, *subkey, value);
    }
    let Some(record_pool) = record_pool else {
        tracing::debug!(key = %key, "no record pool to read the hinted subkeys");
        return;
    };
    let lease = match record_pool
        .acquire(&key, None, rekindle_records::port::OpenIntent::Check)
        .await
    {
        Ok(lease) => lease,
        Err(e) => {
            tracing::warn!(key = %key, error = %e, "hinted record not opened");
            return;
        }
    };
    let read = record_pool.read_hinted(lease, &subkeys).await;
    record_pool.release(lease).await;
    match read {
        Ok(values) => {
            for (subkey, value) in values {
                if inline
                    .as_ref()
                    .is_some_and(|(s, v)| *s == subkey && *v == value.data)
                {
                    continue;
                }
                // Mutual Aid (architecture §14.3): we hold a watch slot for
                // this record, so gossip-relay the notification to community
                // peers who may not; the receivers fetch the new value
                // themselves via `get_dht_value` (no ciphertext crosses
                // gossip).
                relay_watch_change(state, &key, subkey, &value.data);
            }
        }
        Err(e) => tracing::warn!(key = %key, error = %e, "hinted subkeys not read"),
    }
}

fn relay_watch_change(state: &Arc<AppState>, record_key: &str, subkey: u32, value: &[u8]) {
    use rekindle_codec::community::envelope::CommunityEnvelope;

    let community_id_and_pseudonym = {
        let communities = state.communities.read();
        communities.values().find_map(|cs| {
            let matched = cs.anchor.as_deref() == Some(record_key)
                || cs.member_registry_key.as_deref() == Some(record_key)
                || cs.governance_state.as_ref().is_some_and(|gov| {
                    gov.segments
                        .iter()
                        .any(|s| s.anchor == record_key || s.registry_key == record_key)
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
