use rekindle_gossip::mesh::fanout_degree;
use rekindle_protocol::capnp_envelope::{
    encode_community_envelope, encode_signed_envelope, try_decode_community_envelope,
};
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
        encode_community_envelope(envelope).map_err(|e| format!("encode envelope: {e}"))?;
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
    let signed_bytes = encode_signed_envelope(signed);

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
            // A1/P4.1 — enqueue instead of dropping. The next presence poll
            // that lands online peers (presence/poll.rs::presence_poll_tick)
            // drains this queue and re-sends. Bounded at MAX_PENDING (oldest
            // dropped when the cap is hit) so an extended offline burst —
            // e.g. a member joining a community whose creator is offline
            // and producing no peer responses — doesn't OOM the process.
            // Without this, the very first MEK request / governance update
            // / join announcement from a fresh joiner vanished silently and
            // they appeared invisible to everyone else even after coming
            // online.
            const MAX_PENDING: usize = 100;
            drop(communities); // release read lock before taking write
            let mut comms = state.communities.write();
            if let Some(community) = comms.get_mut(community_id) {
                if let Some(g) = community.gossip.as_mut() {
                    if g.pending_mesh_broadcasts.len() >= MAX_PENDING {
                        g.pending_mesh_broadcasts.pop_front();
                    }
                    g.pending_mesh_broadcasts.push_back(signed.clone());
                    tracing::info!(
                        community = %community_id,
                        queued = g.pending_mesh_broadcasts.len(),
                        "send_to_mesh_raw: peers empty, queued for later drain"
                    );
                }
            }
            return;
        }
        gossip
            .peers
            .iter()
            .map(|(key, member)| (key.clone(), member.route_blob.clone()))
            .collect()
    };
    // Mutual Aid (architecture §14.5): rank candidates by reliability
    // (success / (success+failure)) so high-reliability peers — the
    // "ziplines" — are preferred. New peers (no metrics yet) get a
    // neutral score so they still get a chance to prove themselves.
    let scored_peers = sort_peers_by_reliability(state, community_id, peers);
    let degree = fanout_degree(scored_peers.len());
    let selected_peers: Vec<(String, Vec<u8>)> = scored_peers.into_iter().take(degree).collect();

    tracing::info!(
        community = %community_id,
        peer_count = selected_peers.len(),
        fanout_degree = degree,
        "send_to_mesh_raw: sending to gossip fan-out",
    );

    let message_id: Option<String> = try_decode_community_envelope(&signed.envelope_bytes)
        .ok()
        .flatten()
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
                record_peer_reliability(&state_clone, &cid, &pk, true);
                return;
            }
            record_peer_reliability(&state_clone, &cid, &pk, false);

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

/// Rank gossip candidates by reliability (architecture §14.5). Peers
/// with no metrics get a neutral score (0.5) so they aren't permanently
/// shut out. Returns all peers in descending score order.
fn sort_peers_by_reliability(
    state: &SharedState,
    community_id: &str,
    peers: Vec<(String, Vec<u8>)>,
) -> Vec<(String, Vec<u8>)> {
    let scores: std::collections::HashMap<String, f64> = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .map(|cs| {
                cs.peer_reliability
                    .iter()
                    .map(|(k, (s, f))| {
                        let total = f64::from(*s) + f64::from(*f);
                        let score = if total <= 0.0 {
                            0.5
                        } else {
                            f64::from(*s) / total
                        };
                        (k.clone(), score)
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut scored: Vec<(f64, (String, Vec<u8>))> = peers
        .into_iter()
        .map(|(key, blob)| {
            let score = scores.get(&key).copied().unwrap_or(0.5);
            (score, (key, blob))
        })
        .collect();
    // Highest score first; ties broken by peer key for determinism.
    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.1 .0.cmp(&b.1 .0))
    });
    scored.into_iter().map(|(_, peer)| peer).collect()
}

/// Bump a peer's reliability counters. Called from the gossip success/
/// failure paths so the ranking improves over time.
pub fn record_peer_reliability(
    state: &SharedState,
    community_id: &str,
    peer_key: &str,
    success: bool,
) {
    {
        let mut communities = state.communities.write();
        let Some(cs) = communities.get_mut(community_id) else {
            return;
        };
        let entry = cs
            .peer_reliability
            .entry(peer_key.to_string())
            .or_insert((0, 0));
        if success {
            entry.0 = entry.0.saturating_add(1);
        } else {
            entry.1 = entry.1.saturating_add(1);
        }
    }
    // Mark dirty for the next periodic flush (architecture §14.5). The
    // flush task drains this set every 30s; consolidating writes saves
    // ~1000 SQLite upserts/min in busy communities.
    state
        .relay_reliability_dirty
        .lock()
        .insert((community_id.to_string(), peer_key.to_string()));
}

/// Load every community's saved reliability counters from SQLite into
/// the in-memory `peer_reliability` map. Called once on login so the
/// fan-out ranker boots with prior session knowledge instead of
/// treating every peer as neutral.
pub async fn hydrate_peer_reliability(state: &SharedState, pool: &crate::db::DbPool) {
    let owner_key = state_helpers::owner_key_or_default(state);
    if owner_key.is_empty() {
        return;
    }
    let owner = owner_key;
    let rows: Vec<(String, String, i64, i64)> = crate::db_helpers::db_call_or_default(
        pool,
        move |conn| {
            let mut stmt = conn.prepare(
                "SELECT community_id, peer_pseudonym, success_count, failure_count
                 FROM peer_reliability WHERE owner_key = ?1",
            )?;
            let rows = stmt.query_map(rusqlite::params![owner], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        },
    )
    .await;
    if rows.is_empty() {
        return;
    }
    let mut communities = state.communities.write();
    for (community_id, peer, succ, fail) in rows {
        if let Some(cs) = communities.get_mut(&community_id) {
            cs.peer_reliability.insert(
                peer,
                (
                    u32::try_from(succ).unwrap_or(0),
                    u32::try_from(fail).unwrap_or(0),
                ),
            );
        }
    }
}

/// Drain the dirty set and upsert all pending counters in a single DB
/// transaction. Architecture §14.5: in-memory `peer_reliability` is the
/// source of truth during a session; this batch flush just mirrors it
/// to SQLite so the score survives restarts.
pub async fn flush_peer_reliability(state: &crate::state::AppState, pool: &crate::db::DbPool) {
    let owner_key = state
        .identity
        .read()
        .as_ref()
        .map(|id| id.public_key.clone())
        .unwrap_or_default();
    if owner_key.is_empty() {
        return;
    }
    let dirty: Vec<(String, String)> = {
        let mut set = state.relay_reliability_dirty.lock();
        if set.is_empty() {
            return;
        }
        set.drain().collect()
    };
    let snapshot: Vec<(String, String, u32, u32)> = {
        let communities = state.communities.read();
        dirty
            .into_iter()
            .filter_map(|(cid, pk)| {
                communities
                    .get(&cid)
                    .and_then(|cs| cs.peer_reliability.get(&pk))
                    .map(|&(s, f)| (cid, pk, s, f))
            })
            .collect()
    };
    if snapshot.is_empty() {
        return;
    }
    let owner = owner_key;
    let _ = crate::db_helpers::db_call(pool, move |conn| {
        let tx = conn.transaction()?;
        for (cid, pk, s, f) in &snapshot {
            tx.execute(
                "INSERT INTO peer_reliability
                    (owner_key, community_id, peer_pseudonym, success_count, failure_count)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(owner_key, community_id, peer_pseudonym) DO UPDATE SET
                   success_count = excluded.success_count,
                   failure_count = excluded.failure_count",
                rusqlite::params![owner, cid, pk, i64::from(*s), i64::from(*f)],
            )?;
        }
        tx.commit()
    })
    .await;
}

/// Spawn the periodic flush loop. Idempotent — safe to call multiple
/// times; the existing shutdown channel is reused.
pub fn start_peer_reliability_flush(state: SharedState, pool: crate::db::DbPool) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        interval.tick().await; // skip immediate fire
        loop {
            interval.tick().await;
            // Stop if the user has logged out (no active identity).
            if state_helpers::owner_key_or_default(&state).is_empty() {
                break;
            }
            flush_peer_reliability(&state, &pool).await;
        }
    });
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

            let bytes = encode_community_envelope(envelope).unwrap_or_default();
            let mut hash = Blake2b::<U16>::new();
            hash.update(&bytes);
            hex::encode(hash.finalize())
        }
        CommunityEnvelope::WatchRelay {
            record_key,
            subkey,
            content_hash,
            ..
        } => format!("watch:{record_key}:{subkey}:{content_hash}"),
    }
}
