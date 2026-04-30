use rekindle_gossip::mesh::fanout_degree;
use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, SignedEnvelope};
use tauri::Manager as _;

use crate::state::SharedState;
use crate::state_helpers;

pub fn send_to_mesh(
    state: &SharedState,
    community_id: &str,
    envelope: &CommunityEnvelope,
) -> Result<(), String> {
    use rekindle_protocol::dht::community::envelope;

    let my_pseudonym_key = {
        let communities = state.communities.read();
        let community = communities.get(community_id).ok_or("community not found")?;
        community.my_pseudonym_key.clone().unwrap_or_default()
    };

    let signing_key = {
        let secret = state.identity_secret.lock();
        let identity_secret = (*secret).ok_or("identity not unlocked")?;
        rekindle_crypto::group::pseudonym::derive_community_pseudonym(
            &identity_secret,
            community_id,
        )
    };
    let envelope_bytes =
        serde_json::to_vec(envelope).map_err(|e| format!("serialize envelope: {e}"))?;
    let signed = envelope::sign_envelope(
        &signing_key,
        community_id,
        &my_pseudonym_key,
        &envelope_bytes,
    );

    let dedup_key = extract_mesh_dedup_key(envelope);
    state
        .dedup_cache
        .lock()
        .check_and_insert(community_id, &my_pseudonym_key, &dedup_key);

    {
        let mut communities = state.communities.write();
        if let Some(community) = communities.get_mut(community_id) {
            if let Some(ref mut gossip) = community.gossip {
                gossip.lamport_counter += 1;
            }
        }
    }

    send_to_mesh_raw(state, community_id, &signed);
    Ok(())
}

pub fn send_to_mesh_raw(state: &SharedState, community_id: &str, signed: &SignedEnvelope) {
    let Ok(signed_bytes) = serde_json::to_vec(signed) else {
        tracing::warn!(community = %community_id, "send_to_mesh_raw: failed to serialize envelope");
        return;
    };

    let Some(rc) = state_helpers::safe_routing_context(state) else {
        tracing::warn!(community = %community_id, "send_to_mesh_raw: no routing context");
        return;
    };

    let peers: Vec<(String, Vec<u8>)> = {
        let communities = state.communities.read();
        let Some(community) = communities.get(community_id) else {
            tracing::warn!(community = %community_id, "send_to_mesh_raw: community not found");
            return;
        };
        let Some(ref gossip) = community.gossip else {
            tracing::warn!(community = %community_id, "send_to_mesh_raw: gossip overlay is None");
            return;
        };
        if gossip.peers.is_empty() {
            tracing::warn!(community = %community_id, "send_to_mesh_raw: no gossip peers - message will not be delivered");
            return;
        }
        gossip
            .peers
            .iter()
            .map(|(key, member)| (key.clone(), member.route_blob.clone()))
            .collect()
    };
    let degree = fanout_degree(peers.len());
    let selected_peers: Vec<(String, Vec<u8>)> = peers.into_iter().take(degree).collect();

    tracing::info!(
        community = %community_id,
        peer_count = selected_peers.len(),
        fanout_degree = degree,
        "send_to_mesh_raw: sending to gossip fan-out",
    );

    let message_id: Option<String> =
        serde_json::from_slice::<CommunityEnvelope>(&signed.envelope_bytes)
            .ok()
            .and_then(|envelope| match envelope {
                CommunityEnvelope::MessageNotification { message_id, .. } => Some(message_id),
                _ => None,
            });

    for (peer_key, route_blob) in selected_peers {
        let rc = rc.clone();
        let data = signed_bytes.clone();
        let cid = community_id.to_string();
        let state_clone = state.clone();
        let msg_id = message_id.clone();
        let pk = peer_key.clone();
        tokio::spawn(async move {
            let send_result = match rc.api().import_remote_private_route(route_blob) {
                Ok(route_id) => {
                    rc.app_message(veilid_core::Target::RouteId(route_id), data.clone())
                        .await
                }
                Err(e) => Err(veilid_core::VeilidAPIError::generic(e)),
            };

            if send_result.is_ok() {
                if let Some(ref mid) = msg_id {
                    record_delivery(&state_clone, mid, &cid, &pk, "delivered");
                }
                return;
            }

            tracing::info!(community = %cid, peer = %pk, "route stale, attempting DHT re-resolve");
            let fresh_blob = resolve_peer_route_from_db(&state_clone, &cid, &pk).await;
            if let Some(blob) = fresh_blob {
                match rc.api().import_remote_private_route(blob.clone()) {
                    Ok(route_id) => {
                        if let Err(e) = rc
                            .app_message(veilid_core::Target::RouteId(route_id), data)
                            .await
                        {
                            tracing::warn!(community = %cid, peer = %pk, error = %e, "re-resolved route still failed");
                            if let Some(ref mid) = msg_id {
                                record_delivery(&state_clone, mid, &cid, &pk, "failed");
                            }
                        } else {
                            update_peer_route(&state_clone, &cid, &pk, blob);
                            if let Some(ref mid) = msg_id {
                                record_delivery(&state_clone, mid, &cid, &pk, "delivered");
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(community = %cid, peer = %pk, error = %e, "re-resolved route also invalid");
                        if let Some(ref mid) = msg_id {
                            record_delivery(&state_clone, mid, &cid, &pk, "failed");
                        }
                    }
                }
            } else {
                tracing::warn!(community = %cid, peer = %pk, "no fresh route found in DHT");
                if let Some(ref mid) = msg_id {
                    record_delivery(&state_clone, mid, &cid, &pk, "failed");
                }
            }
        });
    }
}

fn record_delivery(
    state: &SharedState,
    message_id: &str,
    community_id: &str,
    recipient: &str,
    status: &str,
) {
    let app_handle = state.app_handle.read().clone();
    if let Some(ref app_handle) = app_handle {
        if let Some(pool) = app_handle.try_state::<crate::db::DbPool>() {
            let mid = message_id.to_string();
            let cid = community_id.to_string();
            let rp = recipient.to_string();
            let st = status.to_string();
            let now = rekindle_utils::timestamp_secs();
            crate::db_helpers::db_fire(&pool, "record_delivery", move |conn| {
                conn.execute(
                    "INSERT INTO message_delivery (message_id, community_id, recipient_pseudonym, status, attempts, last_attempt_at) \
                     VALUES (?1, ?2, ?3, ?4, 1, ?5) \
                     ON CONFLICT(message_id, recipient_pseudonym) \
                     DO UPDATE SET status=excluded.status, attempts=attempts+1, last_attempt_at=excluded.last_attempt_at",
                    rusqlite::params![mid, cid, rp, st, now.cast_signed()],
                )?;
                Ok(())
            });
        }
    }
}

async fn resolve_peer_route_from_db(
    state: &SharedState,
    community_id: &str,
    peer_pseudonym: &str,
) -> Option<Vec<u8>> {
    use rekindle_protocol::dht::community::member_registry;

    let registry_key = {
        let communities = state.communities.read();
        let community = communities.get(community_id)?;
        community.member_registry_key.clone()?
    };

    let app_handle = state.app_handle.read().clone();
    let app_handle = app_handle.as_ref()?;
    let pool = app_handle.try_state::<crate::db::DbPool>()?;
    let cid = community_id.to_string();
    let pk = peer_pseudonym.to_string();
    let subkey_index = crate::db_helpers::db_call(&pool, move |conn| {
        conn.query_row(
            "SELECT subkey_index FROM community_members WHERE community_id = ?1 AND pseudonym_key = ?2",
            rusqlite::params![cid, pk],
            |row| row.get::<_, u32>(0),
        )
        .ok()
        .ok_or_else(|| rusqlite::Error::QueryReturnedNoRows)
    })
    .await
    .ok()?;

    let rc = state_helpers::safe_routing_context(state)?;
    let mgr = rekindle_protocol::dht::DHTManager::new(rc);
    match member_registry::read_member_presence_fresh(&mgr, &registry_key, subkey_index).await {
        Ok(Some(presence)) if presence.status != "offline" => {
            presence.route_blob.filter(|blob| !blob.is_empty())
        }
        _ => None,
    }
}

fn update_peer_route(state: &SharedState, community_id: &str, peer: &str, blob: Vec<u8>) {
    let mut communities = state.communities.write();
    if let Some(community) = communities.get_mut(community_id) {
        if let Some(ref mut gossip) = community.gossip {
            let now = rekindle_utils::timestamp_secs();
            let status = gossip
                .online_members
                .get(peer)
                .map_or_else(|| "online".to_string(), |member| member.status.clone());
            let member = crate::state::OnlineMember {
                route_blob: blob,
                status,
                last_seen: now,
            };
            gossip
                .online_members
                .insert(peer.to_string(), member.clone());
            if gossip.peers.contains_key(peer) {
                gossip.peers.insert(peer.to_string(), member);
            }
        }
    }
}

pub fn extract_mesh_dedup_key(envelope: &CommunityEnvelope) -> String {
    match envelope {
        CommunityEnvelope::MessageNotification { message_id, .. } => message_id.clone(),
        CommunityEnvelope::TypingIndicator {
            channel_id,
            pseudonym_key,
        } => {
            let bucket = rekindle_utils::timestamp_secs() / 5;
            format!("typing:{channel_id}:{pseudonym_key}:{bucket}")
        }
        CommunityEnvelope::PresenceUpdate { pseudonym_key, .. } => {
            let bucket = rekindle_utils::timestamp_secs() / 30;
            format!("presence:{pseudonym_key}:{bucket}")
        }
        CommunityEnvelope::Control(_) => {
            use blake2::{digest::consts::U16, Blake2b, Digest};

            let bytes = serde_json::to_vec(envelope).unwrap_or_default();
            let mut hash = Blake2b::<U16>::new();
            hash.update(&bytes);
            hex::encode(hash.finalize())
        }
    }
}
