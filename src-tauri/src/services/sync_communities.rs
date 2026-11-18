//! A watched community record's change. Nothing here polls (D4): the
//! periodic re-announce and sync requests are the Host's
//! (`adapters::governance::community_sync`, plan E3.1 C2b), and a governance
//! record's ValueChange rebuilds that one community (plan E3.1 C1).

use std::sync::Arc;

use crate::state::AppState;
use crate::state_helpers;
use rekindle_db::Db;

use super::sync_service::request_channel_sync;

/// Route a watched record's change: a governance record to the Host's
/// governance arm, which rebuilds that community
/// alone and opens the channel records it newly names (plan E3.1 C1); a
/// registry to a presence tick; a channel record to its sync. `Ok(false)`
/// for a record no community holds; `NotLoggedIn` while locked.
pub(crate) async fn handle_community_record_change(
    state: &Arc<AppState>,
    pool: &Db,
    dht_key: &str,
    subkeys: &[u32],
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
        .on_watched_change(dht_key, subkeys)
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
            community
                .channel_log_keys
                .iter()
                .find_map(|(channel_id, record_key)| {
                    (record_key == dht_key).then(|| ChangedRecord::Channel {
                        community_id: community.id.clone(),
                        channel_id: channel_id.clone(),
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
