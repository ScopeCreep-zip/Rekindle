//! v2.0 CRDT governance state read/write helpers.

use std::sync::Arc;

use crate::db::DbPool;
use crate::state::AppState;

use super::hex_to_id_16;
use super::node::app_handle;

/// Get a clone of the cached CRDT governance state for a community.
///
/// Returns `None` if the community doesn't exist or governance state isn't loaded yet.
pub fn governance_state(
    state: &Arc<AppState>,
    community_id: &str,
) -> Option<rekindle_governance::state::GovernanceState> {
    state
        .communities
        .read()
        .get(community_id)
        .and_then(|cs| cs.governance_state.clone())
}

/// Update the cached governance state for a community.
///
/// Also syncs `my_role_ids` from the CRDT role assignments so permission
/// checks and SQLite always reflect the latest governance (Bug #6 fix).
///
/// Called after:
/// - GovernanceNotify gossip messages (fast path)
/// - ValueChange DHT watch notifications (consistency path)
/// - Full CRDT merge on join or reconnect
pub fn set_governance_state(
    state: &Arc<AppState>,
    community_id: &str,
    gov_state: rekindle_governance::state::GovernanceState,
) {
    use crate::state::{CategoryInfo, ChannelInfo, ChannelType, RoleDefinition};

    // Set true if the rebuild adds any new watch targets so we spawn a
    // watch_community_records refresh after dropping the write lock.
    let mut needs_watch_refresh = false;

    let mut communities = state.communities.write();
    if let Some(cs) = communities.get_mut(community_id) {
        // ── Sync my_role_ids from CRDT state ──
        let mut is_creator = false;
        if let Some(ref pk_hex) = cs.my_pseudonym_key {
            if let Ok(pk_bytes) = hex::decode(pk_hex) {
                if let Ok(arr) = <[u8; 32]>::try_from(pk_bytes.as_slice()) {
                    let pseudo = rekindle_types::id::PseudonymKey(arr);
                    if let Some(role_ids) = gov_state.role_assignments.get(&pseudo) {
                        cs.my_role_ids = role_ids
                            .iter()
                            .copied()
                            .map(rekindle_types::id::RoleId::to_legacy_u32)
                            .collect();
                        cs.my_role_ids.sort_unstable();
                    }
                    is_creator = gov_state.creator.as_ref() == Some(&pseudo);
                }
            }
        }

        // ── Capture prior watch targets for diff-based refresh (A4/P0.4) ──
        // After the rebuild we compare the new set against this snapshot so
        // we only spawn watch_community_records when keys actually changed.
        // Without this, an admin creating a channel remotely produced an
        // updated cs.channel_log_keys but no watch — followers never received
        // messages on the new channel.
        //
        // Snapshotted from the same sources `tracked_watch_keys` reads —
        // channel logs, Plate Gate segments, channel-segment records —
        // NOT from `open_community_records.channel_keys`. That list means
        // "opened this session", and an earlier version of this function
        // overwrote it with every governance-derived key: the unopened
        // keys then failed their watches with Veilid 'record not open'
        // on every retry tick, while `open_new_channel_records` skipped
        // opening them because they were already "in the list".
        let prior_watch_targets: std::collections::HashSet<String> = cs
            .channel_log_keys
            .values()
            .cloned()
            .chain(cs.governance_state.iter().flat_map(|g| {
                g.segments
                    .iter()
                    .flat_map(|s| [s.governance_key.clone(), s.registry_key.clone()])
                    .chain(
                        g.channel_segment_records
                            .values()
                            .map(|csr| csr.record_key.clone()),
                    )
            }))
            .collect();

        // ── Sync channels from governance ChannelCreated entries ──
        // Build a map of existing unread counts to preserve them
        let existing_unreads: std::collections::HashMap<String, u32> = cs
            .channels
            .iter()
            .map(|ch| (ch.id.clone(), ch.unread_count))
            .collect();
        let existing_record_keys: std::collections::HashMap<String, Option<String>> = cs
            .channels
            .iter()
            .map(|ch| (ch.id.clone(), ch.message_record_key.clone()))
            .collect();
        let existing_notification_levels: std::collections::HashMap<String, String> = cs
            .channels
            .iter()
            .map(|ch| (ch.id.clone(), ch.notification_level.clone()))
            .collect();
        let existing_notification_sound_refs: std::collections::HashMap<String, Option<String>> =
            cs.channels
                .iter()
                .map(|ch| (ch.id.clone(), ch.notification_sound_ref.clone()))
                .collect();

        let mut channel_log_keys = std::collections::HashMap::new();
        let mut channels: Vec<ChannelInfo> = gov_state
            .channels
            .iter()
            .map(|(ch_id, ch)| {
                let id_hex = hex::encode(ch_id.0);
                if !ch.record_key.is_empty() {
                    channel_log_keys.insert(id_hex.clone(), ch.record_key.clone());
                }
                ChannelInfo {
                    unread_count: existing_unreads.get(&id_hex).copied().unwrap_or(0),
                    message_record_key: existing_record_keys
                        .get(&id_hex)
                        .cloned()
                        .flatten()
                        .or_else(|| {
                            if ch.record_key.is_empty() {
                                None
                            } else {
                                Some(ch.record_key.clone())
                            }
                        }),
                    id: id_hex.clone(),
                    name: ch.name.clone(),
                    channel_type: match ch.channel_type.as_str() {
                        "voice" => ChannelType::Voice,
                        "announcement" => ChannelType::Announcement,
                        "forum" => ChannelType::Forum,
                        "stage" => ChannelType::Stage,
                        "directory" => ChannelType::Directory,
                        "media" => ChannelType::Media,
                        "events" => ChannelType::Events,
                        "dm" => ChannelType::Dm,
                        _ => ChannelType::Text,
                    },
                    category_id: ch.category_id.map(|c| hex::encode(c.0)),
                    topic: ch.topic.clone().unwrap_or_default(),
                    forum_tags: ch.forum_tags.clone(),
                    stage_speakers: Vec::new(),
                    stage_moderator: None,
                    slowmode_seconds: ch.slowmode_seconds,
                    nsfw: ch.nsfw.unwrap_or(false),
                    mek_generation: 0,
                    notification_level: existing_notification_levels
                        .get(&id_hex)
                        .cloned()
                        .unwrap_or_else(|| "all".to_string()),
                    notification_sound_ref: existing_notification_sound_refs
                        .get(&id_hex)
                        .cloned()
                        .unwrap_or(None),
                    parent_voice_channel_id: None,
                }
            })
            .collect();
        channels.sort_by(|a, b| {
            let a_pos = gov_state
                .channels
                .get(&rekindle_types::id::ChannelId(hex_to_id_16(&a.id)))
                .map_or(u32::MAX, |ch| ch.position);
            let b_pos = gov_state
                .channels
                .get(&rekindle_types::id::ChannelId(hex_to_id_16(&b.id)))
                .map_or(u32::MAX, |ch| ch.position);
            a_pos.cmp(&b_pos).then_with(|| a.name.cmp(&b.name))
        });
        cs.channels = channels;
        cs.channel_log_keys.clone_from(&channel_log_keys);
        // Deliberately NOT written into `open_community_records.channel_keys`:
        // that list records what was actually opened this session, and
        // `tracked_watch_keys` derives the full watch-target set (channel
        // logs, Plate Gate §15.4 segment records, channel-segment records)
        // from this synced state directly. `watch_record` opens a target
        // itself when the watch reports 'record not open'.

        // ── Sync roles from governance RoleDefinition entries ──
        cs.roles = gov_state
            .roles
            .iter()
            .map(|(rid, r)| RoleDefinition {
                id: rekindle_types::id::RoleId::to_legacy_u32(*rid),
                name: r.name.clone(),
                color: r.color,
                permissions: r.permissions,
                position: r.position.cast_signed(),
                hoist: r.hoist,
                mentionable: r.mentionable,
                self_assignable: r.self_assignable,
                exclusion_group: r.exclusion_group.clone(),
            })
            .collect();
        cs.roles.sort_by_key(|role| role.position);

        // ── Sync categories from governance ──
        cs.categories = gov_state
            .categories
            .iter()
            .map(|(cat_id, cat)| CategoryInfo {
                id: hex::encode(cat_id.0),
                name: cat.name.clone(),
                sort_order: cat.position.cast_signed(),
            })
            .collect();
        cs.categories.sort_by_key(|category| category.sort_order);

        // ── Sync metadata (name, description, icon_hash, banner_hash) ──
        if let Some(ref meta) = gov_state.metadata {
            cs.name.clone_from(&meta.name);
            cs.description.clone_from(&meta.description);
            cs.icon_hash.clone_from(&meta.icon_hash);
            cs.banner_hash.clone_from(&meta.banner_hash);
        }

        // Plan §Failure 4 — creator now holds an Owner role with
        // ADMINISTRATOR perms via the genesis RoleAssignment, so
        // `display_role_name` resolves to "Owner" naturally; no
        // is_creator special-case needed at the read sites.
        let _ = is_creator;

        // ── Sync MEK generation ──
        cs.mek_generation = gov_state.mek_generation;

        // ── Prune banned pseudonyms from gossip overlay (A4/P0.4 — arch §13) ──
        // When a peer is banned via CRDT merge, they must immediately stop
        // receiving our outbound gossip and disappear from local member lists.
        // The merge already excluded them from gov_state.role_assignments etc.,
        // but the gossip overlay (peers, online_members) and known_members are
        // separate state on CommunityState and need explicit pruning here —
        // otherwise send_to_mesh keeps fanning out to a banned ex-member.
        if !gov_state.bans.is_empty() {
            let banned_hex: std::collections::HashSet<String> =
                gov_state.bans.iter().map(|p| hex::encode(p.0)).collect();
            if let Some(gossip) = cs.gossip.as_mut() {
                gossip
                    .peers
                    .retain(|hex_key, _| !banned_hex.contains(hex_key));
                gossip
                    .online_members
                    .retain(|hex_key, _| !banned_hex.contains(hex_key));
            }
            cs.known_members
                .retain(|hex_key| !banned_hex.contains(hex_key));
        }

        // Detect whether any new watch targets appeared so we only spawn
        // watch_community_records when something actually changed (avoids
        // needless Veilid traffic on every governance merge). Compared
        // over the same derived sources the prior snapshot used.
        needs_watch_refresh = cs
            .channel_log_keys
            .values()
            .any(|k| !prior_watch_targets.contains(k))
            || gov_state.segments.iter().any(|s| {
                !prior_watch_targets.contains(&s.governance_key)
                    || !prior_watch_targets.contains(&s.registry_key)
            })
            || gov_state
                .channel_segment_records
                .values()
                .any(|csr| !prior_watch_targets.contains(&csr.record_key));

        cs.governance_state = Some(gov_state);
    }
    drop(communities);

    // ── Sync Lost Cargo pinned-attachment set ──
    // Drives `state.pinned_attachments[community_id]` from the merged
    // governance set so the chunk cache eviction sweep honours the
    // latest AttachmentPinned/unpin entries (architecture §28.9 line 3283).
    // Also opens the per-community chunk cache lazily — login restores
    // and runtime governance updates both flow through this function.
    if let Err(e) = crate::services::community::files::ensure_cache_open(state, community_id) {
        tracing::debug!(community = %community_id, error = %e, "Lost Cargo cache unavailable");
    }
    crate::services::community::files::sync_pinned_from_governance(state, community_id);

    // Plate Gate (architecture §15): open any newly-merged segment SMPL
    // records so subsequent get_dht_value/watch calls work. Spawned because
    // the open is async + uses Veilid I/O; failures are logged but don't
    // block the rest of the merge pipeline.
    let state_for_open = state.clone();
    let community_id_owned = community_id.to_string();
    tokio::spawn(async move {
        crate::services::community::segments::open_new_segments(
            &state_for_open,
            &community_id_owned,
        )
        .await;
    });

    // A4/P0.4 — refresh watches whenever the merge added new watch targets
    // (a remote admin creates a channel, or a Plate Gate segment-N record
    // appears for the first time). Without this, cs.channel_log_keys gets
    // updated but no watch_dht_values is issued, so followers never receive
    // ValueChange events for the new records and the channel stays empty.
    // Idempotent at the Veilid level — already-watched records renew safely.
    if needs_watch_refresh {
        let state_for_watch = state.clone();
        let community_id_for_watch = community_id.to_string();
        tokio::spawn(async move {
            if let Err(e) = crate::services::community::watch::watch_community_records(
                &state_for_watch,
                &community_id_for_watch,
            )
            .await
            {
                tracing::warn!(
                    community = %community_id_for_watch,
                    error = %e,
                    "failed to refresh watches after governance update"
                );
            }
        });
    }

    // Architecture §18.4 + §28.9 line 3286: eager-cache expression assets
    // for any ExpressionAdded entry that just merged in. Spawned for the
    // same reason as open_new_segments — uses Veilid app_call I/O.
    let state_for_eager = state.clone();
    let community_id_eager = community_id.to_string();
    tokio::spawn(async move {
        crate::services::community::expression_assets::eager_fetch_missing(
            &state_for_eager,
            &community_id_eager,
        )
        .await;
    });

    // Architecture §32 W16 — materialise EventCreated governance entries
    // into the SQLite community_events table so late joiners (who saw the
    // entries via DHT merge but missed the live ControlPayload gossip)
    // can still see events in get_events / event reminders / calendar UI.
    if let Some(app_handle) = app_handle(state) {
        use tauri::Manager;
        let pool: tauri::State<'_, DbPool> = app_handle.state();
        crate::services::community::events_hydration::hydrate_events_from_governance(
            state,
            pool.inner(),
            community_id,
        );
        crate::services::community::wake_event_reminders(state);
    }
}

/// Get the governance record DHT key for a community.
pub fn governance_key(state: &Arc<AppState>, community_id: &str) -> Option<String> {
    state
        .communities
        .read()
        .get(community_id)
        .and_then(|cs| cs.governance_key.clone())
}

/// Get the community's current Lamport counter value.
pub fn lamport_counter(state: &Arc<AppState>, community_id: &str) -> u64 {
    state
        .communities
        .read()
        .get(community_id)
        .map_or(0, |cs| cs.lamport_counter)
}

/// Increment the Lamport counter for a community and return the new value.
/// Used on every message send.
pub fn increment_lamport(state: &Arc<AppState>, community_id: &str) -> u64 {
    let mut communities = state.communities.write();
    if let Some(cs) = communities.get_mut(community_id) {
        let mut clock = rekindle_gossip::lamport::LamportClock::new(cs.lamport_counter);
        cs.lamport_counter = clock.increment();
        cs.lamport_counter
    } else {
        0
    }
}

/// Merge a received Lamport timestamp into the community's counter.
/// `counter = max(counter, received) + 1` — standard Lamport merge rule
/// gated by `MAX_LAMPORT_DRIFT` (M9.2). Returns `true` on accept,
/// `false` on drift-reject. Caller should drop the corresponding
/// envelope when this returns `false` so a forged-future Lamport from
/// a malicious peer cannot fast-forward our clock.
pub fn merge_lamport(state: &Arc<AppState>, community_id: &str, received: u64) -> bool {
    let mut communities = state.communities.write();
    if let Some(cs) = communities.get_mut(community_id) {
        let mut clock = rekindle_gossip::lamport::LamportClock::new(cs.lamport_counter);
        match clock.merge(received) {
            Some(advanced) => {
                cs.lamport_counter = advanced;
                true
            }
            None => false,
        }
    } else {
        // Unknown community — let downstream decide; we make no claim
        // about clock state.
        true
    }
}

/// Compute effective permissions for the local user in a community channel.
/// Uses the cached CRDT governance state and returns 0 until governance loads.
pub fn my_permissions(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: Option<&rekindle_types::id::ChannelId>,
) -> u64 {
    let communities = state.communities.read();
    let Some(cs) = communities.get(community_id) else {
        return 0;
    };
    let Some(gov) = &cs.governance_state else {
        return 0;
    };
    let Some(pseudo_hex) = &cs.my_pseudonym_key else {
        return 0;
    };
    // Decode hex pseudonym to PseudonymKey
    let pseudo_bytes: [u8; 32] = match hex::decode(pseudo_hex) {
        Ok(b) if b.len() == 32 => b.try_into().unwrap_or([0u8; 32]),
        _ => return 0,
    };
    let pseudo = rekindle_types::id::PseudonymKey(pseudo_bytes);
    let now = rekindle_utils::timestamp_secs();
    rekindle_governance::permissions::compute_permissions(&pseudo, channel_id, gov, now)
}
