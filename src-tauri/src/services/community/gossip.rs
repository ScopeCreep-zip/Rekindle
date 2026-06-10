//! Phase 20 REDO — thin facade.
//!
//! Mesh-broadcast pipeline (sign + dedup + lamport-bump + peer-select
//! + supervised fan-out + per-peer retry with DHT route re-resolve)
//! lives in `rekindle_gossip::mesh_broadcast`. This module constructs
//! a `GossipAdapter` per call and delegates.
//!
//! Peer-reliability persistence (hydrate from SQLite on login, dirty
//! flush every 30s, drain on logout) stays src-tauri-side: it's pure
//! AppState + DbPool orchestration with no protocol logic worth
//! abstracting behind a trait.

use std::sync::Arc;

use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, SignedEnvelope};

use crate::services::gossip_adapter::deps_impl::build_adapter;
use crate::state::{AppState, SharedState};
use crate::state_helpers;

/// Sign + dedup + bump lamport + fan out. Returns `Err` if the app
/// handle / DbPool can't be acquired; transport failures are
/// best-effort and recorded as reliability + delivery rows inside
/// the orchestrator.
pub fn send_to_mesh(
    state: &SharedState,
    community_id: &str,
    envelope: &CommunityEnvelope,
) -> Result<(), String> {
    let Some(adapter) = build_adapter(state) else {
        return Err("app handle / db pool unavailable".to_string());
    };
    let adapter = Arc::new(adapter);
    let cid = community_id.to_string();
    let env = envelope.clone();
    // Pre-port src-tauri `send_to_mesh` was sync (returned `Result`).
    // The crate orchestrator is async because trait methods are async;
    // every caller already runs on a tokio worker, so spawn the
    // pipeline and return immediately — preserves the prior
    // fire-and-forget semantics (pipeline errors are logged inside).
    tauri::async_runtime::spawn(async move {
        if let Err(error) = rekindle_gossip::send_to_mesh(adapter, &cid, &env).await {
            tracing::warn!(community = %cid, %error, "send_to_mesh: pipeline error");
        }
    });
    Ok(())
}

/// Architecture §10.6 — directed channel-scoped send for voice/video
/// media and its per-stream control traffic. Resolves the recipient
/// set from the active voice session's transport roster (the peers
/// that announced VoiceJoin for this exact channel) and fans the
/// signed envelope out to them with `ttl = 0`, so it is never queued,
/// relayed, or seen by community members outside the channel.
///
/// Errors when the local user is not actively joined to
/// `(community_id, channel_id)` — sending channel media while not in
/// the channel is a caller bug, not a transport condition.
pub fn send_to_channel_peers(
    state: &SharedState,
    community_id: &str,
    channel_id: &str,
    envelope: &CommunityEnvelope,
) -> Result<(), String> {
    let transport = {
        let ve = state.voice_engine.lock();
        let Some(handle) = ve.as_ref() else {
            return Err("not in a voice session".to_string());
        };
        if handle.community_id.as_deref() != Some(community_id) || handle.channel_id != channel_id {
            return Err(format!(
                "active voice session is not bound to {community_id}/{channel_id}"
            ));
        }
        handle.transport.clone()
    };
    let Some(adapter) = build_adapter(state) else {
        return Err("app handle / db pool unavailable".to_string());
    };
    let adapter = Arc::new(adapter);
    let cid = community_id.to_string();
    let ch = channel_id.to_string();
    let env = envelope.clone();
    let state_for_counter = Arc::clone(state);
    // Same fire-and-forget shape as `send_to_mesh`: the roster
    // snapshot needs the tokio transport lock, so peers are resolved
    // inside the spawned task.
    let drop_counter = Arc::clone(&state_for_counter);
    tauri::async_runtime::spawn(async move {
        let (peers, handshake) = {
            let guard = transport.lock().await;
            let peers: Vec<rekindle_gossip::PeerInfo> = guard
                .peer_entries()
                .into_iter()
                .map(|(pseudonym_key, route_blob)| rekindle_gossip::PeerInfo {
                    pseudonym_key,
                    route_blob,
                })
                .collect();
            (peers, guard.handshake())
        };
        if peers.is_empty() {
            // Encoded media with nobody to send to — under the
            // media-ready gate this should never fire in steady state;
            // every occurrence is a roster/handshake regression signal.
            let n = drop_counter
                .channel_send_empty_roster_drops
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                + 1;
            if n == 1 || n.is_multiple_of(30) {
                tracing::warn!(
                    community = %cid,
                    channel = %ch,
                    ?handshake,
                    dropped_total = n,
                    "send_to_channel_peers: empty roster — envelope dropped",
                );
            }
            return;
        }
        if let Err(error) = rekindle_gossip::send_to_channel_peers(adapter, &cid, &env, peers).await
        {
            tracing::warn!(community = %cid, channel = %ch, %error, "send_to_channel_peers: pipeline error");
        }
    });
    Ok(())
}

/// Fan out a pre-signed envelope. Used by the presence-poll drain
/// path which holds pending broadcasts from a prior empty-peers
/// fan-out attempt.
pub fn send_to_mesh_raw(state: &SharedState, community_id: &str, signed: &SignedEnvelope) {
    let Some(adapter) = build_adapter(state) else {
        tracing::warn!(community = %community_id, "send_to_mesh_raw: adapter unavailable");
        return;
    };
    let adapter = Arc::new(adapter);
    rekindle_gossip::send_to_mesh_raw(adapter, community_id, signed.clone());
}

/// Bump a peer's reliability counters and mark dirty for the next
/// periodic flush. Phase 20 — exposed as a standalone src-tauri
/// surface for callers (sync_service retry path, voice signaling
/// failure paths, future cross-track diagnostics) that need to
/// record reliability OUTSIDE the gossip mesh orchestrator. The
/// orchestrator itself records reliability via the
/// `GossipDeps::record_peer_reliability` trait method; this wrapper
/// preserves the pre-port src-tauri call-site shape so plan-mandated
/// future callers don't need an adapter dance.
pub fn record_peer_reliability(
    state: &SharedState,
    community_id: &str,
    peer_key: &str,
    success: bool,
) {
    {
        let mut communities = state.communities.write();
        let Some(community) = communities.get_mut(community_id) else {
            return;
        };
        let entry = community
            .peer_reliability
            .entry(peer_key.to_string())
            .or_insert((0, 0));
        if success {
            entry.0 = entry.0.saturating_add(1);
        } else {
            entry.1 = entry.1.saturating_add(1);
        }
    }
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
    let rows: Vec<(String, String, i64, i64)> =
        crate::db_helpers::db_call_or_default(pool, move |conn| {
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
        })
        .await;
    if rows.is_empty() {
        return;
    }
    let mut communities = state.communities.write();
    for (community_id, peer, succ, fail) in rows {
        if let Some(community) = communities.get_mut(&community_id) {
            community.peer_reliability.insert(
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
pub async fn flush_peer_reliability(state: &AppState, pool: &crate::db::DbPool) {
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
                    .and_then(|c| c.peer_reliability.get(&pk))
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
/// times; the loop self-terminates once the user logs out (empty
/// owner key).
pub fn start_peer_reliability_flush(state: SharedState, pool: crate::db::DbPool) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        interval.tick().await; // skip immediate fire
        loop {
            interval.tick().await;
            if state_helpers::owner_key_or_default(&state).is_empty() {
                break;
            }
            flush_peer_reliability(&state, &pool).await;
        }
    });
}
