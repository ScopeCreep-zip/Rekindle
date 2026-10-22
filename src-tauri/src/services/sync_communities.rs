//! A watched community record's change. Nothing here polls (D4): the
//! periodic re-announce and sync requests are the Host's
//! (`adapters::governance::community_sync`, plan E3.1 C2b), and a governance
//! record's ValueChange rebuilds that one community (plan E3.1 C1).

use std::sync::Arc;

use crate::state::AppState;
use crate::state_helpers;
use rekindle_db::Db;

use super::sync_service::request_channel_sync;

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
