use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use futures::stream::{FuturesUnordered, StreamExt};
use rand::seq::SliceRandom;
use rekindle_protocol::dht::DHTManager;
use tauri::Emitter;

use crate::channels::CommunityEvent;
use crate::state::{AppState, GossipOverlay, OnlineMember};
use crate::state_helpers;

use super::registry::{
    ensure_registry_open, persist_discovered_registry_members, write_our_presence,
};
use super::sync::run_initial_sync;
use crate::services::community::join::try_derive_slot_keypair;

fn presence_event_id_bytes(event_id: &str) -> [u8; 16] {
    let hash = blake3::hash(event_id.as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&hash.as_bytes()[..16]);
    bytes
}

/// Start the 60-second presence poll loop for a community.
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
        if let Err(e) = presence_poll_tick(&state, &community_id).await {
            tracing::warn!(
                community = %community_id,
                error = %e,
                "initial presence poll tick failed"
            );
        }

        let rapid_ticks = 6;
        let mut rapid_interval = tokio::time::interval(std::time::Duration::from_secs(5));
        rapid_interval.tick().await;
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

        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        interval.tick().await;
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

pub async fn presence_poll_tick_public(
    state: &Arc<AppState>,
    community_id: &str,
) -> Result<(), String> {
    presence_poll_tick(state, community_id).await
}

async fn presence_poll_tick(state: &Arc<AppState>, community_id: &str) -> Result<(), String> {
    let rc = state_helpers::safe_routing_context(state).ok_or("not attached")?;
    let mgr = DHTManager::new(rc.clone());

    let registry_key = {
        let communities = state.communities.read();
        let c = communities.get(community_id).ok_or("community not found")?;
        c.member_registry_key.clone()
    };
    let Some(registry_key) = registry_key else {
        tracing::warn!(
            community_id,
            "presence_poll_tick: member_registry_key is None — skipping (join may be pending)"
        );
        return Ok(());
    };

    ensure_registry_open(state, community_id, &mgr, &registry_key).await?;

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

    let slot_keypair_str = if slot_keypair_str.is_none() {
        if let (Some(ref seed_hex), Some(subkey_idx)) = (&slot_seed_hex, my_subkey_index) {
            try_derive_slot_keypair(state, community_id, seed_hex, subkey_idx)
        } else {
            None
        }
    } else {
        slot_keypair_str
    };

    // Compute history ranges for Shared Locker pattern (mutual aid).
    // Advertises which message lamport ranges we have cached locally so
    // newcomers can target us for message catchup.
    let history_ranges = compute_history_ranges(state, community_id).await;

    write_our_presence(
        state,
        community_id,
        &rc,
        &registry_key,
        &my_pseudonym,
        my_subkey_index,
        slot_keypair_str.as_ref(),
        slot_seed_hex.is_some(),
        history_ranges,
    )
    .await;

    let now_secs = rekindle_utils::timestamp_secs();
    let stale_threshold = now_secs.saturating_sub(180);
    let banned_members = state_helpers::governance_state(state, community_id)
        .map(|gov_state| {
            gov_state
                .bans
                .iter()
                .map(|pseudo| hex::encode(pseudo.0))
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();

    let mut online_members: HashMap<String, OnlineMember> = HashMap::new();
    let mut known_member_keys: HashSet<String> = HashSet::new();
    let mut discovered_members: Vec<super::registry::DiscoveredRow> = Vec::new();

    {
        let scan_rc =
            state_helpers::safe_routing_context(state).ok_or("not attached for presence scan")?;
        let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(10));
        let my_subkey = my_subkey_index.unwrap_or(u32::MAX);
        let my_segment_index = {
            let communities = state.communities.read();
            communities
                .get(community_id)
                .and_then(|c| c.my_segment_index)
                .unwrap_or(0)
        };

        // Plate Gate (architecture §15.5): scan every segment's registry
        // record. The implicit segment 0 + each `SegmentAdded` discovered
        // in governance state. Each segment is its own SMPL record holding
        // 255 LOCAL subkeys; presence rows are tagged with the segment they
        // came from so SQLite persistence + re-derivation stay correct.
        let descriptors =
            crate::services::community::segments::segment_descriptors(state, community_id);

        let mut futs = FuturesUnordered::new();
        for descriptor in &descriptors {
            let reg_key = match descriptor.registry_key.parse::<veilid_core::RecordKey>() {
                Ok(k) => k,
                Err(e) => {
                    tracing::warn!(
                        community = %community_id,
                        segment = descriptor.segment_index,
                        registry_key = %descriptor.registry_key,
                        error = %e,
                        "presence scan: invalid registry key — skipping segment"
                    );
                    continue;
                }
            };
            for subkey in 0u32..255u32 {
                if descriptor.segment_index == my_segment_index && subkey == my_subkey {
                    continue;
                }
                let sem = sem.clone();
                let rc = scan_rc.clone();
                let rk = reg_key.clone();
                let seg_idx = descriptor.segment_index;
                futs.push(async move {
                    let permit = sem.acquire().await.unwrap();
                    let result = rc.get_dht_value(rk, subkey, false).await;
                    drop(permit);
                    (seg_idx, subkey, result)
                });
            }
        }
        // Reference the legacy single-registry parameter for diagnostics
        // until C1-2 unifies the registry-key trail; this also silences
        // the `registry_key` shadowing warning during the staged rollout.
        let _ = registry_key;

        while let Some((seg_idx, subkey, result)) = futs.next().await {
            match result {
                Ok(Some(val)) if !val.data().is_empty() => {
                    if let Ok(presence) = serde_json::from_slice::<
                        rekindle_types::presence::MemberPresence,
                    >(val.data())
                    {
                        // Architecture §26 W26 — drop unsigned/forged
                        // presence rows. The slot keypair is shared, so
                        // any member could otherwise impersonate any
                        // other (RSVPs, voice channel, custom status).
                        let sig_arr: [u8; 64] =
                            match presence.signature.as_slice().try_into() {
                                Ok(arr) => arr,
                                Err(_) => continue,
                            };
                        if rekindle_secrets::derive::verify_pseudonym_signature(
                            &presence.pseudonym_key.0,
                            &presence.signing_bytes(),
                            &sig_arr,
                        )
                        .is_err()
                        {
                            tracing::warn!(
                                "presence subkey rejected: bad pseudonym signature",
                            );
                            continue;
                        }
                        let pk = hex::encode(presence.pseudonym_key.0);
                        if banned_members.contains(&pk) {
                            tracing::trace!(member = %pk, "presence scan: ignoring banned member");
                            continue;
                        }
                        known_member_keys.insert(pk.clone());
                        discovered_members.push((seg_idx, subkey, presence.clone()));

                        if presence.status == "offline" {
                        } else if presence.last_heartbeat <= stale_threshold {
                            tracing::trace!(member = %pk, "presence scan: stale heartbeat");
                        } else if presence.route_blob.is_empty() {
                            tracing::trace!(member = %pk, "presence scan: empty route_blob");
                        } else {
                            online_members.insert(
                                pk,
                                OnlineMember {
                                    route_blob: presence.route_blob,
                                    status: presence.status,
                                    last_seen: now_secs,
                                },
                            );
                        }
                    }
                }
                _ => {}
            }
        }
    }

    update_known_member_state(
        state,
        community_id,
        &discovered_members,
        known_member_keys,
        &banned_members,
        &my_pseudonym,
    );
    update_event_rsvp_state(state, community_id, &discovered_members, &my_pseudonym).await;
    update_member_profiles(state, community_id, &discovered_members);

    {
        let communities = state.communities.read();
        if let Some(cs) = communities.get(community_id) {
            if let Some(ref gossip) = cs.gossip {
                let eviction_threshold = now_secs.saturating_sub(180);
                for (pk, member) in &gossip.online_members {
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

    let n = online_members.len();
    let d = crate::state::gossip_degree(n);
    let selected = random_peer_sample(&online_members, d);

    let offline_members = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .and_then(|cs| cs.gossip.as_ref())
            .map(|gossip| {
                gossip
                    .online_members
                    .keys()
                    .filter(|pk| !online_members.contains_key(*pk) && *pk != &my_pseudonym)
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };

    let needs_sync = rebuild_gossip_and_drain_pending(
        state,
        community_id,
        selected,
        online_members,
        n,
    );

    if !offline_members.is_empty() {
        if let Some(app_handle) = state_helpers::app_handle(state) {
            for pseudonym_key in offline_members {
                let _ = app_handle.emit(
                    "community-event",
                    CommunityEvent::MemberPresenceChanged {
                        community_id: community_id.to_string(),
                        pseudonym_key,
                        status: "offline".to_string(),
                        game_name: None,
                        game_id: None,
                        elapsed_seconds: None,
                        server_address: None,
                    },
                );
            }
        }
    }

    tracing::info!(
        community = %community_id,
        online_members = n,
        gossip_degree = d,
        needs_sync,
        "presence_poll_tick: gossip overlay updated"
    );

    if n > 0 {
        let stale_syncs: Vec<(String, u32)> = {
            let communities = state.communities.read();
            communities.get(community_id).map_or(vec![], |cs| {
                cs.pending_syncs
                    .iter()
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
            let sync_envelope =
                rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
                    rekindle_protocol::dht::community::envelope::ControlPayload::SyncRequest {
                        channel_id: channel_id.clone(),
                        since_timestamp: 0,
                    },
                );
            let _ = crate::services::community::send_to_mesh(state, community_id, &sync_envelope);
            let mut communities = state.communities.write();
            if let Some(cs) = communities.get_mut(community_id) {
                cs.pending_syncs
                    .insert(channel_id.clone(), (now_secs, attempt + 1));
            }
        }
        {
            let mut communities = state.communities.write();
            if let Some(cs) = communities.get_mut(community_id) {
                cs.pending_syncs.retain(|_, (_, count)| *count < 3);
            }
        }
    }

    if needs_sync {
        run_initial_sync(state, community_id, d).await;
    }

    // A5/P4.3 — Plate Gate auto-expand. If we're an admin and the highest
    // segment has filled up (every slot occupied), spawn a background task
    // that creates the next segment. Without this, the 256th member of any
    // community is permanently rejected with "Community is full" until an
    // admin manually invokes expand_community_segment from the UI.
    // Spawned because the expand involves multiple Veilid I/Os (create
    // record × 2, write SegmentAdded entry) and we don't want to block
    // the rest of the poll cycle.
    maybe_auto_expand_segment(state, community_id);

    Ok(())
}

/// A5/P4.3 — admin-side trigger for Plate Gate segment expansion.
///
/// Architecture §15.1: trigger condition is "all 255 slots in the highest
/// segment occupied". Architecture §15.2: action is "any admin writes a
/// SegmentAdded governance entry". Per the v2.0 plan, a non-admin joiner
/// hitting a full registry returns the existing "Community is full" error
/// — they cannot self-elect to expand. This helper covers the admin side.
fn maybe_auto_expand_segment(state: &Arc<AppState>, community_id: &str) {
    use rekindle_protocol::dht::community::permissions_v2::Permissions;
    if crate::commands::community::require_permission(
        state,
        community_id,
        Permissions::MANAGE_COMMUNITY,
    )
    .is_err()
    {
        return; // not an admin — wait for one to come online
    }
    let state_clone = state.clone();
    let cid = community_id.to_string();
    tokio::spawn(async move {
        match super::super::segments::highest_segment_full(&state_clone, &cid).await {
            Ok(true) => {
                tracing::info!(
                    community = %cid,
                    "highest segment is full — auto-expanding (admin trigger)"
                );
                if let Err(e) =
                    super::super::segments::expand_community_segment(&state_clone, &cid).await
                {
                    tracing::warn!(
                        community = %cid,
                        error = %e,
                        "auto segment expansion failed — next admin's poll will retry"
                    );
                }
            }
            Ok(false) => {} // segment has open slots; no expansion needed
            Err(e) => {
                tracing::debug!(
                    community = %cid,
                    error = %e,
                    "highest_segment_full check failed — skipping expansion"
                );
            }
        }
    });
}

fn update_member_profiles(
    state: &Arc<AppState>,
    community_id: &str,
    discovered_members: &[super::registry::DiscoveredRow],
) {
    let mut profiles_changed = false;
    {
        let mut communities = state.communities.write();
        let Some(community) = communities.get_mut(community_id) else {
            return;
        };
        for (_segment_index, _subkey, presence) in discovered_members {
            let pseudonym_hex = hex::encode(presence.pseudonym_key.0);
            let next = crate::state::MemberProfileSnapshot {
                display_name: presence.display_name.clone(),
                bio: presence.bio.clone(),
                pronouns: presence.pronouns.clone(),
                theme_color: presence.theme_color,
                badges: presence.badges.clone(),
                avatar_ref: presence.avatar_ref.clone(),
                banner_ref: presence.banner_ref.clone(),
            };
            // Wave 5 / D1 — only flag the frontend refresh if a field
            // actually changed. Without this, every 30 s presence
            // poll would emit `MembersRefreshed`, which the frontend
            // treats as a re-fetch trigger — burning IPC bandwidth
            // for no visual delta.
            let prev = community.member_profiles.get(&pseudonym_hex);
            if prev.is_none_or(|existing| existing != &next) {
                profiles_changed = true;
                community.member_profiles.insert(pseudonym_hex, next);
            }
        }
    }

    // Wave 5 / D1 — surface profile updates to every frontend (Tauri
    // GUI today; future CLI/TUI subscribers via the same event stream)
    // so member panels re-render with the latest bio/pronouns/avatar/
    // banner/theme/badges. The frontend already listens for
    // `MembersRefreshed` and re-calls `getCommunityMembers`, which
    // returns the merged profile snapshot from `member_profiles`.
    if profiles_changed {
        if let Some(app_handle) = state_helpers::app_handle(state) {
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::MembersRefreshed {
                    community_id: community_id.to_string(),
                },
            );
        }
    }
}

async fn update_event_rsvp_state(
    state: &Arc<AppState>,
    community_id: &str,
    discovered_members: &[super::registry::DiscoveredRow],
    my_pseudonym: &str,
) {
    let known_events = load_known_event_ids(state, community_id).await;
    let event_by_presence_id: HashMap<[u8; 16], String> = known_events
        .into_iter()
        .map(|event_id| (presence_event_id_bytes(&event_id), event_id))
        .collect();

    let mut aggregated: HashMap<String, Vec<crate::state::EventRsvpEntry>> = HashMap::new();
    for (_segment_index, _subkey, presence) in discovered_members {
        let pseudonym_key = hex::encode(presence.pseudonym_key.0);
        for rsvp in &presence.event_rsvps {
            if let Some(event_id) = event_by_presence_id.get(&rsvp.event_id.0) {
                aggregated.entry(event_id.clone()).or_default().push(
                    crate::state::EventRsvpEntry {
                        pseudonym_key: pseudonym_key.clone(),
                        status: rsvp.status.clone(),
                    },
                );
            }
        }
    }

    let local_rsvps = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .map(|community| community.my_event_rsvps.clone())
            .unwrap_or_default()
    };
    for (event_id, status) in local_rsvps {
        aggregated
            .entry(event_id.clone())
            .or_default()
            .retain(|entry| entry.pseudonym_key != my_pseudonym);
        aggregated
            .entry(event_id)
            .or_default()
            .push(crate::state::EventRsvpEntry {
                pseudonym_key: my_pseudonym.to_string(),
                status,
            });
    }

    for rsvps in aggregated.values_mut() {
        rsvps.sort_by(|a, b| a.pseudonym_key.cmp(&b.pseudonym_key));
    }

    let mut communities = state.communities.write();
    if let Some(community) = communities.get_mut(community_id) {
        community.event_rsvps_by_event = aggregated;
    }
}

async fn load_known_event_ids(state: &Arc<AppState>, community_id: &str) -> Vec<String> {
    use tauri::Manager as _;

    let Some(app_handle) = state_helpers::app_handle(state) else {
        return Vec::new();
    };
    let pool: tauri::State<'_, crate::db::DbPool> = app_handle.state();
    let Ok(owner_key) = state_helpers::current_owner_key(state) else {
        return Vec::new();
    };
    let community_id = community_id.to_string();
    pool.call(move |conn| {
        let mut stmt = conn.prepare(
            "SELECT id FROM community_events WHERE owner_key = ?1 AND community_id = ?2",
        )?;
        let rows = stmt
            .query_map(rusqlite::params![owner_key, community_id], |row| {
                row.get::<_, String>(0)
            })?
            .filter_map(Result::ok)
            .collect::<Vec<_>>();
        Ok::<Vec<String>, rusqlite::Error>(rows)
    })
    .await
    .unwrap_or_default()
}

fn random_peer_sample(
    online: &HashMap<String, OnlineMember>,
    d: usize,
) -> HashMap<String, OnlineMember> {
    if d == 0 || online.is_empty() {
        return HashMap::new();
    }
    if d >= online.len() {
        return online.clone();
    }

    let keys: Vec<&String> = online.keys().collect();
    let mut rng = rand::rngs::OsRng;
    let selected: Vec<&String> = keys.choose_multiple(&mut rng, d).copied().collect();

    selected
        .into_iter()
        .filter_map(|k| online.get(k).map(|v| (k.clone(), v.clone())))
        .collect()
}

/// A1/P4.1 — atomic gossip-overlay rebuild + pending-broadcast drain.
///
/// Combines the previously-inline write-lock rebuild and the post-drop
/// drain so `presence_poll_tick` stays under the workspace `too_many_lines`
/// budget. Returns whether a sync request should follow (was previously
/// `needs_initial_sync` AND we now have peers).
///
/// Lock order: takes `state.communities.write()` only for the rebuild,
/// drops it, then re-calls `send_to_mesh_raw` (which takes its own read
/// lock) for each drained envelope. Releasing between phases is required
/// because `send_to_mesh_raw` would otherwise deadlock on the same lock.
fn rebuild_gossip_and_drain_pending(
    state: &Arc<AppState>,
    community_id: &str,
    selected: HashMap<String, crate::state::OnlineMember>,
    online_members: HashMap<String, crate::state::OnlineMember>,
    n: usize,
) -> bool {
    let (needs_sync, drained_pending) = {
        let mut communities = state.communities.write();
        let Some(cs) = communities.get_mut(community_id) else {
            return false;
        };
        let counter = cs.gossip.as_ref().map_or(0, |g| g.lamport_counter);
        let was_needs_sync = cs.gossip.as_ref().is_none_or(|g| g.needs_initial_sync);
        // Preserve pending_mesh_broadcasts across the rebuild so a queue
        // accumulated while peers was empty isn't wiped. Drain when peers
        // are about to become non-empty so we can re-send post-rebuild.
        let mut pending: std::collections::VecDeque<
            rekindle_protocol::dht::community::envelope::SignedEnvelope,
        > = cs
            .gossip
            .as_mut()
            .map(|g| std::mem::take(&mut g.pending_mesh_broadcasts))
            .unwrap_or_default();
        let will_have_peers = !selected.is_empty();
        let drained = if will_have_peers {
            std::mem::take(&mut pending)
        } else {
            std::collections::VecDeque::new()
        };
        cs.gossip = Some(GossipOverlay {
            peers: selected,
            online_members,
            lamport_counter: counter,
            needs_initial_sync: was_needs_sync,
            pending_mesh_broadcasts: pending,
        });
        (was_needs_sync && n > 0, drained)
    };

    if !drained_pending.is_empty() {
        tracing::info!(
            community = %community_id,
            queued = drained_pending.len(),
            "presence_poll_tick: draining pending mesh broadcasts now that peers are online"
        );
        for envelope in drained_pending {
            super::super::gossip::send_to_mesh_raw(state, community_id, &envelope);
        }
    }

    needs_sync
}

fn update_known_member_state(
    state: &Arc<AppState>,
    community_id: &str,
    discovered_members: &[super::registry::DiscoveredRow],
    known_member_keys: HashSet<String>,
    banned_members: &HashSet<String>,
    my_pseudonym: &str,
) -> HashMap<String, Vec<u32>> {
    let mut member_roles = current_member_roles(state, community_id);
    merge_discovered_roles(state, community_id, discovered_members, &mut member_roles);
    merge_my_roles(state, community_id, my_pseudonym, &mut member_roles);

    {
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(community_id) {
            for banned in banned_members {
                cs.known_members.remove(banned);
                cs.member_roles.remove(banned);
                if let Some(ref mut gossip) = cs.gossip {
                    gossip.online_members.remove(banned);
                    gossip.peers.remove(banned);
                }
            }
            cs.known_members.extend(known_member_keys);
            cs.member_roles.clone_from(&member_roles);
        }
    }

    persist_discovered_registry_members(
        state,
        community_id,
        discovered_members,
        &member_roles,
        banned_members,
    );
    member_roles
}

fn current_member_roles(state: &Arc<AppState>, community_id: &str) -> HashMap<String, Vec<u32>> {
    let communities = state.communities.read();
    communities
        .get(community_id)
        .map(|cs| cs.member_roles.clone())
        .unwrap_or_default()
}

fn merge_discovered_roles(
    state: &Arc<AppState>,
    community_id: &str,
    discovered_members: &[super::registry::DiscoveredRow],
    member_roles: &mut HashMap<String, Vec<u32>>,
) {
    if let Some(gov_state) = state_helpers::governance_state(state, community_id) {
        for (_segment_index, _subkey, presence) in discovered_members {
            let pseudonym_hex = hex::encode(presence.pseudonym_key.0);
            let role_ids = gov_state
                .role_assignments
                .get(&presence.pseudonym_key)
                .map_or_else(|| vec![0], role_ids_from_governance);
            member_roles.insert(pseudonym_hex, role_ids);
        }
    }
}

fn role_ids_from_governance(assigned: &HashSet<rekindle_types::id::RoleId>) -> Vec<u32> {
    let mut ids: Vec<u32> = assigned
        .iter()
        .map(|role_id| u32::from_le_bytes([role_id.0[0], role_id.0[1], role_id.0[2], role_id.0[3]]))
        .collect();
    ids.sort_unstable();
    ids
}

fn merge_my_roles(
    state: &Arc<AppState>,
    community_id: &str,
    my_pseudonym: &str,
    member_roles: &mut HashMap<String, Vec<u32>>,
) {
    if my_pseudonym.is_empty() {
        return;
    }
    let communities = state.communities.read();
    let my_roles = communities
        .get(community_id)
        .map_or_else(|| vec![0], |cs| cs.my_role_ids.clone());
    member_roles.insert(my_pseudonym.to_string(), my_roles);
}

/// Compute history ranges for the Shared Locker pattern.
///
/// Queries SQLite for min/max lamport_ts per channel for this community.
/// Returns empty Vec if no messages stored or DB unavailable. The query is
/// a simple `MIN/MAX GROUP BY` on indexed columns — sub-millisecond cost.
async fn compute_history_ranges(
    state: &Arc<AppState>,
    community_id: &str,
) -> Vec<rekindle_types::presence::HistoryRange> {
    use tauri::Manager as _;

    let Some(app_handle) = state_helpers::app_handle(state) else {
        return Vec::new();
    };
    let pool: tauri::State<'_, crate::db::DbPool> = app_handle.state();
    let Ok(owner_key) = state_helpers::current_owner_key(state) else {
        return Vec::new();
    };
    let cid = community_id.to_string();

    let result: Result<Vec<(String, u64, u64)>, tokio_rusqlite::Error> = pool
        .call(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT conversation_id, MIN(lamport_ts), MAX(lamport_ts) \
                 FROM messages \
                 WHERE owner_key = ?1 AND community_id = ?2 AND lamport_ts > 0 \
                 GROUP BY conversation_id",
            )?;
            let rows = stmt
                .query_map(rusqlite::params![owner_key, cid], |row| {
                    let channel_id_str: String = row.get(0)?;
                    let oldest: u64 = row.get(1)?;
                    let newest: u64 = row.get(2)?;
                    Ok((channel_id_str, oldest, newest))
                })?
                .filter_map(Result::ok)
                .collect::<Vec<_>>();
            Ok(rows)
        })
        .await;

    match result {
        Ok(rows) => rows
            .into_iter()
            .map(|(channel_id_str, oldest, newest)| {
                // Convert string channel_id to ChannelId ([u8; 16])
                let bytes: [u8; 16] = hex::decode(&channel_id_str)
                    .ok()
                    .and_then(|b| <[u8; 16]>::try_from(b.as_slice()).ok())
                    .unwrap_or_else(|| {
                        // Fallback: pad/truncate raw string bytes to 16
                        let mut buf = [0u8; 16];
                        let src = channel_id_str.as_bytes();
                        let len = src.len().min(16);
                        buf[..len].copy_from_slice(&src[..len]);
                        buf
                    });
                rekindle_types::presence::HistoryRange {
                    channel_id: rekindle_types::id::ChannelId(bytes),
                    oldest_lamport: oldest,
                    newest_lamport: newest,
                }
            })
            .collect(),
        Err(e) => {
            tracing::trace!(
                community = %community_id,
                error = %e,
                "failed to compute history ranges — will advertise empty"
            );
            Vec::new()
        }
    }
}
