use std::sync::Arc;

use tauri::Manager as _;

use rekindle_protocol::dht::community::types::MemberSummary;
use rekindle_protocol::dht::community::member_registry;
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

/// Sync discovered members into `known_members` (in-memory) and `community_members` (SQLite).
/// Called from both `join_community` and `presence_poll_tick` to ensure members are recognized.
pub(crate) fn sync_members_to_state_and_db(
    state: &Arc<AppState>,
    community_id: &str,
    members: &[MemberSummary],
) {
    // Add to known_members + member_roles so we accept their messages and can check permissions.
    // Also update our own my_role_ids if we find ourselves in the member list.
    {
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(community_id) {
            let my_pseudonym = cs.my_pseudonym_key.clone();
            for member in members {
                cs.known_members.insert(member.pseudonym_key.clone());
                cs.member_roles.insert(member.pseudonym_key.clone(), member.role_ids.clone());

                // Update our own role_ids if we find ourselves in the registry
                if my_pseudonym.as_deref() == Some(&member.pseudonym_key)
                    && !member.role_ids.is_empty()
                    && member.role_ids.len() >= cs.my_role_ids.len()
                {
                    cs.my_role_ids.clone_from(&member.role_ids);
                }
            }
        }
    }

    // Persist to SQLite so get_community_members returns them
    let app_handle_clone = state.app_handle.read().clone();
    if let Some(ref ah) = app_handle_clone {
        let pool: tauri::State<'_, crate::db::DbPool> = ah.state();
        let owner_key = crate::state_helpers::current_owner_key(state).unwrap_or_default();
        for member in members {
            let ok = owner_key.clone();
            let cid = community_id.to_string();
            let pk = member.pseudonym_key.clone();
            let dn = member.display_name.clone();
            let rids = serde_json::to_string(&member.role_ids)
                .unwrap_or_else(|_| "[0,1]".to_string());
            crate::db_helpers::db_fire(
                pool.inner(),
                "persist discovered member",
                move |conn| {
                    conn.execute(
                        "INSERT INTO community_members (owner_key, community_id, pseudonym_key, display_name, role_ids, joined_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, 0) \
                         ON CONFLICT(owner_key, community_id, pseudonym_key) \
                         DO UPDATE SET display_name=excluded.display_name, role_ids=excluded.role_ids",
                        rusqlite::params![ok, cid, pk, dn, rids],
                    )?;
                    Ok(())
                },
            );
        }
    }
}

/// Single presence poll tick.
async fn presence_poll_tick(state: &Arc<AppState>, community_id: &str) -> Result<(), String> {
    let rc = state_helpers::routing_context(state).ok_or("not attached")?;
    let mgr = DHTManager::new(rc);

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

    // Ensure registry record is open (may be closed after restart).
    // Try to open writable if we have the registry owner keypair, otherwise read-only.
    // Note: veilid's open_dht_record overwrites the writer on re-open, so opening
    // read-only here would downgrade a previous writable open. Use the owner keypair
    // if available to preserve write access for coordinator operations.
    {
        let registry_kp = {
            let communities = state.communities.read();
            communities.get(community_id).and_then(|c| c.registry_owner_keypair.clone())
        };
        let opened = if let Some(ref kp_str) = registry_kp {
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
    }

    // Gather our state (clone out before .await)
    let (my_pseudonym, my_subkey_index, slot_keypair_str, slot_seed_hex, is_coordinator) = {
        let communities = state.communities.read();
        let c = communities.get(community_id).ok_or("community not found")?;
        (
            c.my_pseudonym_key.clone().unwrap_or_default(),
            c.my_subkey_index,
            c.slot_keypair.clone(),
            c.slot_seed.clone(),
            c.coordinator_pseudonym.as_ref() == c.my_pseudonym_key.as_ref()
                && c.my_pseudonym_key.is_some(),
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
        let presence = rekindle_protocol::dht::community::types::MemberPresence {
            pseudonym_key: my_pseudonym.clone(),
            status: "online".to_string(),
            status_message: None,
            game_info: None,
            route_blob: our_route_blob,
            last_heartbeat: rekindle_utils::timestamp_secs(),
            is_coordinator,
            coordinator_since: 0,
            is_archiver: false,
        };
        // Parse the Veilid KeyPair from its string representation
        if let Ok(writer_kp) = kp_str.parse::<veilid_core::KeyPair>() {
            if let Err(e) = member_registry::write_member_presence(
                &mgr, &registry_key, subkey_idx, &presence, writer_kp,
            ).await {
                tracing::debug!(
                    community = %community_id,
                    subkey = subkey_idx,
                    error = %e,
                    "failed to write our presence to DHT registry"
                );
            } else {
                tracing::trace!(
                    community = %community_id,
                    subkey = subkey_idx,
                    "wrote presence to DHT registry"
                );
            }
        } else {
            tracing::warn!(
                community = %community_id,
                "failed to parse slot keypair — cannot write presence"
            );
        }
    } else {
        // If we still don't have slot_keypair at this point, we're missing either:
        // - slot_seed (never received JoinAccepted with it, or older coordinator)
        // - my_subkey_index (never assigned a slot in the registry)
        // Try to read my_subkey_index from the DHT member registry as a last resort.
        if my_subkey_index.is_none() && slot_seed_hex.is_some() {
            let members_result = member_registry::read_member_index(&mgr, &registry_key).await;
            if let Ok(members) = members_result {
                if let Some(m) = members.iter().find(|m| m.pseudonym_key == my_pseudonym) {
                    let idx = m.subkey_index;
                    let mut communities = state.communities.write();
                    if let Some(c) = communities.get_mut(community_id) {
                        c.my_subkey_index = Some(idx);
                    }
                    tracing::info!(
                        community = %community_id,
                        subkey = idx,
                        "recovered my_subkey_index from DHT registry — will derive slot keypair next tick"
                    );
                }
            }
        }
        tracing::warn!(
            community = %community_id,
            has_slot_keypair = slot_keypair_str.is_some(),
            has_subkey_index = my_subkey_index.is_some(),
            has_slot_seed = slot_seed_hex.is_some(),
            "cannot write presence — missing slot keypair or subkey index"
        );
    }

    // 2. Read all member entries
    let members = member_registry::read_member_index(&mgr, &registry_key)
        .await
        .map_err(|e| format!("read member index: {e}"))?;

    // Sync all indexed members to known_members + SQLite
    sync_members_to_state_and_db(state, community_id, &members);

    let now_secs = rekindle_utils::timestamp_secs();
    let stale_threshold = now_secs.saturating_sub(300); // 5 minutes

    // Scan presences in parallel — build online members map.
    // Each task gets its own DHTManager (cheap wrapper around Arc-based RoutingContext).
    // Bounded to 10 concurrent DHT reads to avoid overwhelming the network.
    let mut online_members: std::collections::HashMap<String, crate::state::OnlineMember> =
        std::collections::HashMap::new();
    {
        use futures::stream::{FuturesUnordered, StreamExt};

        let rc = state_helpers::routing_context(state).ok_or("not attached for presence scan")?;
        let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(10));
        let mut futs = FuturesUnordered::new();

        for member in &members {
            if member.pseudonym_key == my_pseudonym {
                continue;
            }
            let sem = sem.clone();
            let rc = rc.clone();
            let rk = registry_key.clone();
            let pk = member.pseudonym_key.clone();
            let idx = member.subkey_index;
            futs.push(async move {
                let permit = sem.acquire().await.unwrap();
                let task_mgr = DHTManager::new(rc);
                let result = member_registry::read_member_presence_fresh(&task_mgr, &rk, idx).await;
                drop(permit); // release semaphore slot after DHT read completes
                (pk, result)
            });
        }

        while let Some((pk, result)) = futs.next().await {
            match result {
                Ok(Some(presence)) => {
                    if presence.status == "offline" {
                        tracing::trace!(member = %pk, "presence scan: member is offline");
                    } else if presence.last_heartbeat <= stale_threshold {
                        tracing::debug!(
                            member = %pk,
                            heartbeat = presence.last_heartbeat,
                            threshold = stale_threshold,
                            "presence scan: member heartbeat is stale"
                        );
                    } else if presence.route_blob.is_none() {
                        tracing::info!(
                            member = %pk,
                            "presence scan: member has no route_blob — cannot reach them"
                        );
                    } else if presence.route_blob.as_ref().is_some_and(Vec::is_empty) {
                        tracing::info!(
                            member = %pk,
                            "presence scan: member has empty route_blob — cannot reach them"
                        );
                    } else if let Some(blob) = presence.route_blob {
                        online_members.insert(pk, crate::state::OnlineMember {
                            route_blob: blob,
                            last_seen: now_secs,
                        });
                    }
                }
                Ok(None) => {
                    tracing::trace!(member = %pk, "presence scan: no presence written yet");
                }
                Err(e) => {
                    tracing::debug!(member = %pk, error = %e, "presence scan: failed to read");
                }
            }
        }
    }

    // 3. Discover unindexed members (self-registered joiners not yet in member index)
    discover_unindexed_members(
        state, community_id, &mgr, &registry_key,
        &members, &my_pseudonym, stale_threshold, &mut online_members,
    ).await;

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
async fn discover_unindexed_members(
    state: &Arc<AppState>,
    community_id: &str,
    mgr: &DHTManager,
    registry_key: &str,
    members: &[MemberSummary],
    my_pseudonym: &str,
    stale_threshold: u64,
    online_members: &mut std::collections::HashMap<String, crate::state::OnlineMember>,
) {
    // Build set of indexed slot indices so we only scan unindexed slots
    let indexed_slots: std::collections::HashSet<u32> =
        members.iter().map(|m| m.subkey_index).collect();
    let indexed_keys: std::collections::HashSet<&str> =
        members.iter().map(|m| m.pseudonym_key.as_str()).collect();
    // Scan a window of slots beyond the highest indexed slot to find new joiners.
    // Full 255-slot scan is too expensive (each = DHT network read).
    // New joiners get assigned slots sequentially, so scanning 10 beyond the max
    // covers most cases. Falls back to 10 if no members indexed yet.
    let max_indexed = members.iter().map(|m| m.subkey_index).max().unwrap_or(0);
    let scan_limit = (max_indexed + 10).min(member_registry::SLOTS_PER_SEGMENT);
    let has_registry_kp = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .is_some_and(|cs| cs.registry_owner_keypair.is_some())
    };
    let app_handle = state.app_handle.read().clone();

    for slot in 1..=scan_limit {
        // Skip slots that are already in the member index
        if indexed_slots.contains(&slot) {
            continue;
        }

        match member_registry::read_member_presence_fresh(mgr, registry_key, slot).await {
            Ok(Some(presence)) if !presence.pseudonym_key.is_empty() => {
                if indexed_keys.contains(presence.pseudonym_key.as_str())
                    || presence.pseudonym_key == my_pseudonym
                {
                    continue;
                }

                tracing::info!(
                    community = %community_id,
                    pseudonym = %presence.pseudonym_key,
                    slot = slot,
                    "discovered unindexed member via SMPL presence scan"
                );

                // Add to gossip overlay if online
                if presence.status != "offline" && presence.last_heartbeat > stale_threshold {
                    if let Some(blob) = presence.route_blob {
                        if !blob.is_empty() {
                            let now = rekindle_utils::timestamp_secs();
                            online_members.insert(presence.pseudonym_key.clone(), crate::state::OnlineMember {
                                route_blob: blob,
                                last_seen: now,
                            });
                        }
                    }
                }

                // Add to known_members
                {
                    let mut communities = state.communities.write();
                    if let Some(cs) = communities.get_mut(community_id) {
                        cs.known_members.insert(presence.pseudonym_key.clone());
                    }
                }

                // Emit MemberDiscovered to frontend for ALL members
                if let Some(ref app) = app_handle {
                    use tauri::Emitter as _;
                    let _ = app.emit(
                        "community-event",
                        crate::channels::CommunityEvent::MemberDiscovered {
                            community_id: community_id.to_string(),
                            pseudonym_key: presence.pseudonym_key.clone(),
                            display_name: presence.pseudonym_key.clone(),
                            subkey_index: slot,
                        },
                    );
                }

                // If admin, formally add to member index
                if has_registry_kp {
                    let display_name = presence.pseudonym_key.clone();
                    let state = state.clone();
                    let cid = community_id.to_string();
                    let pk = presence.pseudonym_key.clone();
                    tokio::spawn(async move {
                        handle_discovered_member(&state, &cid, &pk, &display_name, slot).await;
                    });
                }
            }
            _ => {
                // Empty slot or read error — continue scanning.
                // Slots may be non-contiguous (multi-use invites with slot ranges).
            }
        }
    }
}

/// Add a newly discovered (unindexed) member to the member registry and broadcast.
///
/// Called by `presence_poll_tick` when an admin discovers a SMPL presence on a slot
/// that isn't in the member index. Adds them to the index + broadcasts `MemberJoined`.
async fn handle_discovered_member(
    state: &Arc<AppState>,
    community_id: &str,
    pseudonym_key: &str,
    display_name: &str,
    claimed_slot: u32,
) {
    use rekindle_protocol::dht::community::envelope::{ControlPayload, CommunityEnvelope};

    match crate::services::coordinator::state_manager::add_member_to_registry(
        state, community_id, pseudonym_key, display_name, Some(claimed_slot),
    )
    .await
    {
        Ok(idx) => {
            tracing::info!(
                community = %community_id,
                pseudonym = %pseudonym_key,
                subkey_index = idx,
                "lazy discovery: added unindexed member to registry"
            );
            let joined = CommunityEnvelope::Control(ControlPayload::MemberJoined {
                pseudonym_key: pseudonym_key.to_string(),
                display_name: display_name.to_string(),
                role_ids: vec![0, 1],
            });
            crate::services::coordinator::state_manager::broadcast_via_gossip(
                state, community_id, &joined,
            );
            crate::services::coordinator::state_manager::emit_local_member_joined(
                state, community_id, pseudonym_key, display_name,
            );
        }
        Err(e) => {
            tracing::debug!(
                community = %community_id,
                pseudonym = %pseudonym_key,
                error = %e,
                "lazy discovery: failed to add member to registry (may already be indexed)"
            );
        }
    }
}

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
