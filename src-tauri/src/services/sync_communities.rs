//! Phase 23.D.4 — community-side sync helpers extracted from
//! `sync_service.rs` to keep that file under the 500-LoC cap.
//! Mesh-presence re-announce + per-channel watermark sync. Governance is
//! never polled (D4): a governance record's ValueChange rebuilds that one
//! community (plan E3.1 C1).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::state::AppState;
use crate::state_helpers;
use rekindle_db::Db;

use super::sync_service::request_channel_sync;

/// Sync communities by re-announcing our mesh presence.
pub(super) async fn sync_communities(
    state: &Arc<AppState>,
    pool: &Db,
    stop: &tokio_util::sync::CancellationToken,
) -> Result<(), String> {
    if state_helpers::safe_routing_context(state).is_none() {
        return Ok(()); // Not connected yet
    }

    let communities_with_governance = state_helpers::communities_with_governance_keys(state);
    for (community_id, _) in &communities_with_governance {
        if stop.is_cancelled() {
            return Ok(());
        }
        sync_community_channels(state, pool, community_id, stop).await;
        if stop.is_cancelled() {
            return Ok(());
        }
        if let Err(e) = crate::services::community::rejoin_community(state, community_id).await {
            tracing::trace!(community = %community_id, error = %e, "community rejoin failed");
        }
    }

    tracing::debug!(
        communities = communities_with_governance.len(),
        "community sync complete"
    );
    Ok(())
}

/// Route a watched record's change: a governance record (or a Plate Gate
/// segment's) to the Host's governance arm, which rebuilds that community
/// alone and opens the channel records it newly names (plan E3.1 C1); a
/// registry to a presence tick; a channel record to its sync. `Ok(false)`
/// for a record no community holds; `NotLoggedIn` while locked.
pub(crate) async fn handle_community_record_change(
    state: &Arc<AppState>,
    pool: &Db,
    dht_key: &str,
) -> Result<bool, rekindle_db::NotLoggedIn> {
    enum ChangedRecord {
        Registry {
            community_id: String,
        },
        Channel {
            community_id: String,
            channel_id: String,
        },
    }

    if state_helpers::host_governance(state)?
        .on_watched_change(dht_key)
        .await
    {
        return Ok(true);
    }

    let changed = {
        let communities = state.communities.read();
        communities.values().find_map(|community| {
            if community.member_registry_key.as_deref() == Some(dht_key) {
                return Some(ChangedRecord::Registry {
                    community_id: community.id.clone(),
                });
            }
            if let Some(found) =
                community
                    .channel_log_keys
                    .iter()
                    .find_map(|(channel_id, record_key)| {
                        (record_key == dht_key).then(|| ChangedRecord::Channel {
                            community_id: community.id.clone(),
                            channel_id: channel_id.clone(),
                        })
                    })
            {
                return Some(found);
            }
            // Plate Gate (§15.4) — segment-N records route to the same
            // arms as their segment-0 counterparts: a segment-registry
            // change is a presence change, a channel-segment record change
            // is channel traffic (a segment-governance change went to the
            // Host above). These are watched (tracked_watch_keys)
            // and previously fell through to the friend-presence
            // handler, which silently ignored them.
            let gov = community.governance_state.as_ref()?;
            if gov.segments.iter().any(|s| s.registry_key == dht_key) {
                return Some(ChangedRecord::Registry {
                    community_id: community.id.clone(),
                });
            }
            gov.channel_segment_records
                .iter()
                .find_map(|((channel_id, _segment), csr)| {
                    (csr.record_key == dht_key).then(|| ChangedRecord::Channel {
                        community_id: community.id.clone(),
                        channel_id: hex::encode(channel_id.0),
                    })
                })
        })
    };

    match changed {
        Some(ChangedRecord::Registry { community_id }) => {
            let _ =
                crate::services::community::presence_poll_tick_public(state, &community_id).await;
            Ok(true)
        }
        Some(ChangedRecord::Channel {
            community_id,
            channel_id,
        }) => {
            request_channel_sync(state, pool, &community_id, &channel_id).await;
            Ok(true)
        }
        None => Ok(false),
    }
}

fn report_fingerprint(seqs: &[Option<u32>]) -> u64 {
    let mut hasher = DefaultHasher::new();
    seqs.hash(&mut hasher);
    hasher.finish()
}

async fn sync_community_channels(
    state: &Arc<AppState>,
    pool: &Db,
    community_id: &str,
    stop: &tokio_util::sync::CancellationToken,
) {
    let Ok(record_pool) = state_helpers::record_pool(state) else {
        return;
    };
    let channel_pairs = {
        let communities = state.communities.read();
        let Some(cs) = communities.get(community_id) else {
            return;
        };
        cs.channel_log_keys
            .iter()
            .map(|(channel_id, record_key)| (channel_id.clone(), record_key.clone()))
            .collect::<Vec<_>>()
    };

    for (channel_id, record_key) in channel_pairs {
        if stop.is_cancelled() {
            return;
        }
        let report = match record_pool
            .inspect_once(
                &record_key,
                None,
                rekindle_records::port::ReportScope::UpdateGet,
            )
            .await
        {
            Ok(report) => report,
            Err(e) => {
                tracing::trace!(
                    community = %community_id,
                    channel = %channel_id,
                    error = %e,
                    "channel record inspect failed"
                );
                continue;
            }
        };
        let fingerprint = report_fingerprint(&report.network_seqs);
        let should_request_sync = {
            let mut communities = state.communities.write();
            let Some(cs) = communities.get_mut(community_id) else {
                return;
            };
            let previous = cs
                .open_community_records
                .channel_report_fingerprints
                .insert(channel_id.clone(), fingerprint);
            previous.is_some_and(|prev| prev != fingerprint)
        };
        if should_request_sync {
            request_channel_sync(state, pool, community_id, &channel_id).await;
        }
    }
}
