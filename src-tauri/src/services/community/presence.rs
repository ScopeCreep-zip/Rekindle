use std::sync::Arc;

use tauri::Manager as _;

use rekindle_protocol::dht::DHTManager;

use crate::state::{AppState, GossipOverlay};
use crate::state_helpers;

use super::join::try_derive_slot_keypair;

/// Start the 60-second presence poll loop for a community.
///
/// The poll loop:
/// 1. Writes our signed presence to the registry
/// 2. Reads all member presences to discover who is online
/// 3. Updates the gossip overlay peer set (random D peers from online members)
/// 4. Writes coordinator heartbeat if we are coordinator
/// 5. Checks coordinator liveness if we are NOT coordinator
pub fn start_presence_poll(state: Arc<AppState>, community_id: String) {
    use tokio::sync::mpsc;

    let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
    {
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(&community_id) {
            cs.presence_poll_shutdown_tx = Some(shutdown_tx);
        }
    }
    tokio::spawn(async move {
        // Run an immediate first tick so gossip overlay is populated right away
        if let Err(e) = presence_poll_tick(&state, &community_id).await {
            tracing::warn!(
                community = %community_id,
                error = %e,
                "initial presence poll tick failed"
            );
        }

        // Rapid discovery phase: poll every 5s for the first 30s after join.
        // This reduces the peer discovery blind window from 60s to ~10s.
        // After the rapid phase, drop to normal 60s interval.
        let rapid_ticks = 6; // 6 × 5s = 30s rapid phase
        let mut rapid_interval = tokio::time::interval(std::time::Duration::from_secs(5));
        rapid_interval.tick().await; // consume immediate tick
        for tick_num in 0..rapid_ticks {
            tokio::select! {
                _ = rapid_interval.tick() => {
                    if let Err(e) = presence_poll_tick(&state, &community_id).await {
                        tracing::trace!(
                            community = %community_id,
                            tick = tick_num + 1,
                            error = %e,
                            "rapid presence poll tick failed"
                        );
                    }
                }
                _ = shutdown_rx.recv() => return,
            }
        }

        // Normal phase: poll every 60s
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        interval.tick().await; // consume immediate tick
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if let Err(e) = presence_poll_tick(&state, &community_id).await {
                        tracing::debug!(
                            community = %community_id,
                            error = %e,
                            "presence poll tick failed"
                        );
                    }
                }
                _ = shutdown_rx.recv() => break,
            }
        }
    });
}

/// Public entry point for triggering an immediate presence poll tick.
/// Called after SlotKeypairGrant to speed up peer discovery.
pub async fn presence_poll_tick_public(
    state: &Arc<AppState>,
    community_id: &str,
) -> Result<(), String> {
    presence_poll_tick(state, community_id).await
}


/// Single presence poll tick.
async fn presence_poll_tick(state: &Arc<AppState>, community_id: &str) -> Result<(), String> {
    let rc = state_helpers::routing_context(state).ok_or("not attached")?;
    let mgr = DHTManager::new(rc.clone());

    // Read member registry to scan presences
    let registry_key = {
        let communities = state.communities.read();
        let c = communities.get(community_id).ok_or("community not found")?;
        c.member_registry_key.clone()
    };
    let Some(registry_key) = registry_key else {
        tracing::warn!(community_id, "presence_poll_tick: member_registry_key is None — skipping (join may be pending)");
        return Ok(());
    };

    // Ensure registry record is open.
    // If records were already opened during join (CommunityRecords.records_open), skip re-opening
    // to avoid clobbering the writer. Veilid's open_dht_record overwrites the writer unconditionally,
    // so re-opening read-only would destroy write access established during join.
    // Only re-open after restart when records_open is false.
    {
        let records_open = {
            let communities = state.communities.read();
            communities.get(community_id).is_some_and(|c| c.open_community_records.records_open)
        };
        if !records_open {
            // After app restart: records need to be re-opened.
            // Use the registry writer keypair if available to preserve write access.
            let (registry_kp, slot_kp) = {
                let communities = state.communities.read();
                let c = communities.get(community_id);
                (
                    c.and_then(|c| c.registry_owner_keypair.clone()),
                    c.and_then(|c| c.slot_keypair.clone()),
                )
            };
            let writer_kp = registry_kp.or(slot_kp);
            let opened = if let Some(ref kp_str) = writer_kp {
                if let Ok(kp) = kp_str.parse::<veilid_core::KeyPair>() {
                    mgr.open_record_writable(&registry_key, kp).await.is_ok()
                } else {
                    false
                }
            } else {
                false
            };
            if !opened {
                if let Err(e) = mgr.open_record(&registry_key).await {
                    tracing::debug!(community = %community_id, error = %e, "presence_poll: failed to open registry");
                    return Ok(());
                }
            }
            // Track the reopened record and mark as open
            state_helpers::track_open_records(state, std::slice::from_ref(&registry_key));
            {
                let mut communities = state.communities.write();
                if let Some(cs) = communities.get_mut(community_id) {
                    cs.open_community_records.registry_key = Some(registry_key.clone());
                    cs.open_community_records.registry_writer = writer_kp;
                    cs.open_community_records.records_open = true;
                }
            }
            tracing::debug!(community = %community_id, "presence_poll: re-opened registry after restart");
        }
    }

    // Gather our state (clone out before .await)
    let (my_pseudonym, my_subkey_index, slot_keypair_str, slot_seed_hex) = {
        let communities = state.communities.read();
        let c = communities.get(community_id).ok_or("community not found")?;
        (
            c.my_pseudonym_key.clone().unwrap_or_default(),
            c.my_subkey_index,
            c.slot_keypair.clone(),
            c.slot_seed.clone(),
        )
    };

    // Self-heal: if we have slot_seed + my_subkey_index but no slot_keypair,
    // derive it locally. No coordinator needed.
    let slot_keypair_str = if slot_keypair_str.is_none() {
        if let (Some(ref seed_hex), Some(subkey_idx)) = (&slot_seed_hex, my_subkey_index) {
            try_derive_slot_keypair(state, community_id, seed_hex, subkey_idx)
        } else {
            None
        }
    } else {
        slot_keypair_str
    };

    // 1. WRITE our signed presence to the registry (so others can discover our route)
    if let (Some(subkey_idx), Some(ref kp_str)) = (my_subkey_index, &slot_keypair_str) {
        let our_route_blob = state_helpers::our_route_blob(state);
        if our_route_blob.is_none() {
            tracing::warn!(
                community = %community_id,
                "presence_poll_tick: our_route_blob is None — peers cannot reach us"
            );
        }

        // Write MemberPresence directly to our SMPL subkey.
        // With o_cnt:0, subkey index = slot index, no offset.
        let presence = rekindle_types::presence::MemberPresence {
            pseudonym_key: rekindle_types::id::PseudonymKey(
                hex::decode(&my_pseudonym)
                    .ok()
                    .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
                    .unwrap_or([0u8; 32]),
            ),
            display_name: Some(state_helpers::identity_display_name(state)),
            status: "online".into(),
            route_blob: our_route_blob.unwrap_or_default(),
            last_heartbeat: rekindle_utils::timestamp_secs(),
            ..Default::default()
        };
        if let Ok(writer_kp) = kp_str.parse::<veilid_core::KeyPair>() {
            let presence_bytes = serde_json::to_vec(&presence).unwrap_or_default();
            if let Ok(reg_key) = registry_key.parse::<veilid_core::RecordKey>() {
                let write_opts = veilid_core::SetDHTValueOptions {
                    writer: Some(writer_kp),
                    ..Default::default()
                };
                if let Err(e) = rc.set_dht_value(reg_key, subkey_idx, presence_bytes, Some(write_opts)).await {
                    tracing::debug!(
                        community = %community_id,
                        subkey = subkey_idx,
                        error = %e,
                        "failed to write presence to registry"
                    );
                }
            }
        }
    } else {
        tracing::warn!(
            community = %community_id,
            has_slot_keypair = slot_keypair_str.is_some(),
            has_subkey_index = my_subkey_index.is_some(),
            has_slot_seed = slot_seed_hex.is_some(),
            "cannot write presence — missing slot keypair or subkey index"
        );
    }

    let now_secs = rekindle_utils::timestamp_secs();
    let stale_threshold = now_secs.saturating_sub(300); // 5 minutes

    // 2. Scan all 255 registry subkeys — build online members map.
    // With o_cnt:0, each subkey holds a member's MemberPresence directly.
    // No member index needed — the occupied subkeys ARE the member list.
    // Bounded to 10 concurrent reads to avoid overwhelming the DHT.
    let mut online_members: std::collections::HashMap<String, crate::state::OnlineMember> =
        std::collections::HashMap::new();
    let mut known_member_keys: std::collections::HashSet<String> = std::collections::HashSet::new();
    {
        use futures::stream::{FuturesUnordered, StreamExt};

        let scan_rc = state_helpers::routing_context(state).ok_or("not attached for presence scan")?;
        let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(10));
        let my_subkey = my_subkey_index.unwrap_or(u32::MAX);

        let reg_key = registry_key
            .parse::<veilid_core::RecordKey>()
            .map_err(|e| format!("invalid registry key for scan: {e}"))?;

        let mut futs = FuturesUnordered::new();
        for subkey in 0..255u32 {
            if subkey == my_subkey {
                continue; // skip our own subkey
            }
            let sem = sem.clone();
            let rc = scan_rc.clone();
            let rk = reg_key.clone();
            futs.push(async move {
                let permit = sem.acquire().await.unwrap();
                let result = rc.get_dht_value(rk, subkey, false).await;
                drop(permit);
                (subkey, result)
            });
        }

        while let Some((_subkey, result)) = futs.next().await {
            match result {
                Ok(Some(val)) if !val.data().is_empty() => {
                    // Try to deserialize as v2.0 MemberPresence
                    if let Ok(presence) = serde_json::from_slice::<rekindle_types::presence::MemberPresence>(val.data()) {
                        let pk = hex::encode(presence.pseudonym_key.0);
                        known_member_keys.insert(pk.clone());

                        if presence.status == "offline" {
                            // offline — don't add to online members
                        } else if presence.last_heartbeat <= stale_threshold {
                            tracing::trace!(member = %pk, "presence scan: stale heartbeat");
                        } else if presence.route_blob.is_empty() {
                            tracing::trace!(member = %pk, "presence scan: empty route_blob");
                        } else {
                            online_members.insert(pk, crate::state::OnlineMember {
                                route_blob: presence.route_blob,
                                last_seen: now_secs,
                            });
                        }
                    }
                }
                _ => {} // empty subkey or read error — skip
            }
        }
    }

    // Update known_members in state
    {
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(community_id) {
            cs.known_members.extend(known_member_keys);
        }
    }

    // Merge existing online_members with newly scanned ones.
    // Peers discovered via PresenceUpdate (gossip) should survive the DHT scan cycle —
    // they are already verified (signed envelope) and have valid route blobs.
    // Only the DHT scan can REMOVE stale peers (offline status or stale heartbeat).
    {
        let communities = state.communities.read();
        if let Some(cs) = communities.get(community_id) {
            if let Some(ref gossip) = cs.gossip {
                let eviction_threshold = now_secs.saturating_sub(180); // 3 poll intervals
                for (pk, member) in &gossip.online_members {
                    // Keep existing peer if: not already in scan results, not self,
                    // and not stale (seen within 180s)
                    if !online_members.contains_key(pk)
                        && pk != &my_pseudonym
                        && member.last_seen > eviction_threshold
                    {
                        online_members.insert(pk.clone(), member.clone());
                    }
                }
            }
        }
    }

    // Select D random gossip peers
    let n = online_members.len();
    let d = crate::state::gossip_degree(n);
    let selected = random_peer_sample(&online_members, d);

    // Update gossip overlay
    let needs_sync = {
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(community_id) {
            let counter = cs.gossip.as_ref().map_or(0, |g| g.lamport_counter);
            let was_needs_sync = cs.gossip.as_ref().is_none_or(|g| g.needs_initial_sync);
            cs.gossip = Some(GossipOverlay {
                peers: selected,
                online_members,
                lamport_counter: counter,
                needs_initial_sync: was_needs_sync,
            });
            was_needs_sync && n > 0 // Only trigger sync if peers are online
        } else {
            false
        }
    };

    tracing::info!(
        community = %community_id,
        online_members = n,
        gossip_degree = d,
        needs_sync,
        "presence_poll_tick: gossip overlay updated"
    );

    // Retry stale sync requests (older than 60s, max 3 attempts)
    if n > 0 {
        let stale_syncs: Vec<(String, u32)> = {
            let communities = state.communities.read();
            communities.get(community_id).map_or(vec![], |cs| {
                cs.pending_syncs.iter()
                    .filter(|(_, (ts, count))| now_secs.saturating_sub(*ts) > 60 && *count < 3)
                    .map(|(ch, (_, count))| (ch.clone(), *count))
                    .collect()
            })
        };
        for (channel_id, attempt) in &stale_syncs {
            tracing::info!(
                community = %community_id,
                channel = %channel_id,
                attempt = attempt + 1,
                "retrying stale SyncRequest"
            );
            let sync_envelope = rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
                rekindle_protocol::dht::community::envelope::ControlPayload::SyncRequest {
                    channel_id: channel_id.clone(),
                    since_timestamp: 0, // Request everything — recipient filters
                },
            );
            let _ = crate::commands::community::send_to_mesh(state, community_id, &sync_envelope);
            let mut communities = state.communities.write();
            if let Some(cs) = communities.get_mut(community_id) {
                cs.pending_syncs.insert(channel_id.clone(), (now_secs, attempt + 1));
            }
        }
        // Evict syncs that exceeded max attempts
        {
            let mut communities = state.communities.write();
            if let Some(cs) = communities.get_mut(community_id) {
                cs.pending_syncs.retain(|_, (_, count)| *count < 3);
            }
        }
    }

    // Broadcast presence + trigger sync on first successful poll
    if needs_sync {
        run_initial_sync(state, community_id, d).await;
    }

    Ok(())
}

/// Broadcast presence and trigger initial sync (SyncRequest + DHTLog catch-up).
///
/// Called once on the first successful presence poll tick that discovers online peers.
/// Broadcasts our route via PresenceUpdate, sends SyncRequests per channel, and
/// reads DHTLog tails for catch-up. Clears `needs_initial_sync` when done.
async fn run_initial_sync(state: &Arc<AppState>, community_id: &str, d: usize) {
    // Broadcast our presence to gossip peers so they learn our route immediately.
    // Skip if route_blob is None — pointless broadcast, peers can't reach us anyway.
    if d > 0 {
        let (my_pk, our_route) = {
            let communities = state.communities.read();
            let cs = communities.get(community_id);
            (
                cs.and_then(|c| c.my_pseudonym_key.clone()).unwrap_or_default(),
                state_helpers::our_route_blob(state),
            )
        };
        if our_route.is_some() {
            let presence_envelope =
                rekindle_protocol::dht::community::envelope::CommunityEnvelope::PresenceUpdate {
                    pseudonym_key: my_pk,
                    status: "online".to_string(),
                    game_info: None,
                    route_blob: our_route,
                };
            let _ = crate::commands::community::send_to_mesh(state, community_id, &presence_envelope);
        } else {
            tracing::warn!(
                community = %community_id,
                "skipping PresenceUpdate broadcast — route_blob not yet available"
            );
            // Don't clear needs_initial_sync — will retry when route becomes available
            return;
        }
    }

    // Collect all channel IDs for sync
    let all_channel_ids: Vec<String> = {
        let communities = state.communities.read();
        communities.get(community_id)
            .map(|cs| cs.channels.iter().map(|ch| ch.id.clone()).collect())
            .unwrap_or_default()
    };

    let app_handle_clone = state.app_handle.read().clone();
    let owner_key = crate::state_helpers::current_owner_key(state).unwrap_or_default();
    if let Some(ref app_handle) = app_handle_clone {
        let pool: tauri::State<'_, crate::db::DbPool> = app_handle.state();

        for ch_id in &all_channel_ids {
            let ok = owner_key.clone();
            let ch = ch_id.clone();
            let last_ts: i64 = crate::db_helpers::db_call(pool.inner(), move |conn| {
                conn.query_row(
                    "SELECT COALESCE(MAX(timestamp), 0) FROM messages \
                     WHERE owner_key=? AND conversation_id=? AND conversation_type='channel'",
                    rusqlite::params![ok, ch],
                    |r| r.get(0),
                )
            }).await.unwrap_or(0);

            let sync_req = rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
                rekindle_protocol::dht::community::envelope::ControlPayload::SyncRequest {
                    channel_id: ch_id.clone(),
                    since_timestamp: last_ts.cast_unsigned(),
                },
            );
            let _ = crate::commands::community::send_to_mesh(state, community_id, &sync_req);
            // Record pending sync for retry tracking
            let now = rekindle_utils::timestamp_secs();
            let mut communities = state.communities.write();
            if let Some(cs) = communities.get_mut(community_id) {
                cs.pending_syncs.insert(ch_id.clone(), (now, 1));
            }
        }
    }

    // SMPL channel record catch-up — read all member subkeys for each channel
    let channel_entries: Vec<(String, String)> = {
        let communities = state.communities.read();
        communities.get(community_id)
            .map(|cs| cs.channel_log_keys.iter()
                .map(|(ch_id, record_key)| (ch_id.clone(), record_key.clone()))
                .collect())
            .unwrap_or_default()
    };
    let member_count = {
        let communities = state.communities.read();
        communities.get(community_id)
            .map_or(0, |cs| u32::try_from(cs.known_members.len()).unwrap_or(255))
    };

    if !channel_entries.is_empty() && member_count > 0 {
        if let Some(rc) = state_helpers::routing_context(state) {
            for (ch_id, record_key) in &channel_entries {
                match rekindle_protocol::dht::community::channel_record::read_all_channel_messages(
                    &rc, record_key, member_count,
                ).await {
                    Ok(messages) if !messages.is_empty() => {
                        tracing::debug!(
                            community = %community_id,
                            channel = %ch_id,
                            count = messages.len(),
                            "caught up from SMPL channel record"
                        );
                        if let Some(ref app_handle) = app_handle_clone {
                            let pool: tauri::State<'_, crate::db::DbPool> = app_handle.state();
                            let channel = ch_id.clone();
                            let ok = owner_key.clone();
                            crate::db_helpers::db_fire(pool.inner(), "smpl_channel_catchup", move |conn| {
                                for msg in &messages {
                                    let mid = msg.message_id.as_deref().unwrap_or("");
                                    if mid.is_empty() { continue; }
                                    let exists: bool = conn.query_row(
                                        "SELECT EXISTS(SELECT 1 FROM messages WHERE owner_key=?1 AND message_id=?2)",
                                        rusqlite::params![ok, mid],
                                        |r| r.get(0),
                                    ).unwrap_or(false);
                                    if exists { continue; }
                                    let _ = conn.execute(
                                        "INSERT OR IGNORE INTO messages \
                                         (owner_key, conversation_id, conversation_type, sender_key, body, timestamp, message_id, lamport_ts) \
                                         VALUES (?1, ?2, 'channel', ?3, ?4, ?5, ?6, ?7)",
                                        rusqlite::params![
                                            ok, channel, msg.sender_pseudonym,
                                            String::from_utf8_lossy(&msg.ciphertext),
                                            msg.timestamp, mid, msg.lamport_ts,
                                        ],
                                    );
                                }
                                Ok(())
                            });
                        }
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::debug!(
                            community = %community_id,
                            channel = %ch_id,
                            error = %e,
                            "SMPL channel catch-up failed"
                        );
                    }
                }
            }
        }
    }

    // Mark sync as done
    let mut communities = state.communities.write();
    if let Some(cs) = communities.get_mut(community_id) {
        if let Some(ref mut g) = cs.gossip {
            g.needs_initial_sync = false;
        }
    }
    tracing::info!(community = %community_id, "initial sync complete");
}

/// Scan SMPL slots beyond the member index to discover self-registered joiners.
///
/// Self-service joiners write SMPL presence to their claimed slot but may not
/// be in the member index yet. This scans a window of extra slots and adds any
/// discovered members to the gossip overlay (and to the index if we're an admin).
/// Select D random peers from the online members map.
fn random_peer_sample(
    online: &std::collections::HashMap<String, crate::state::OnlineMember>,
    d: usize,
) -> std::collections::HashMap<String, crate::state::OnlineMember> {
    use rand::seq::SliceRandom;

    if d == 0 || online.is_empty() {
        return std::collections::HashMap::new();
    }
    if d >= online.len() {
        return online.clone();
    }

    let keys: Vec<&String> = online.keys().collect();
    let mut rng = rand::rngs::OsRng;
    let selected: Vec<&String> = keys
        .choose_multiple(&mut rng, d)
        .copied()
        .collect();

    selected
        .into_iter()
        .filter_map(|k| online.get(k).map(|v| (k.clone(), v.clone())))
        .collect()
}
