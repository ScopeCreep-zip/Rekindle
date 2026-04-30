use std::collections::HashMap;
use std::sync::Arc;

use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_sync::history::{select_best_peer, HistoryAd};
use rekindle_types::presence::MemberPresence;

use crate::state::AppState;
use crate::state_helpers;

pub(super) fn schedule_history_catchup(state: Arc<AppState>, community_id: String) {
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(15)).await;
        if let Err(error) = request_history_catchup(&state, &community_id).await {
            tracing::debug!(community = %community_id, error = %error, "history ad catchup skipped");
        }
    });
}

async fn request_history_catchup(state: &Arc<AppState>, community_id: &str) -> Result<(), String> {
    let rc = state_helpers::safe_routing_context(state).ok_or("not attached")?;
    let registry_key = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .and_then(|community| community.member_registry_key.clone())
            .ok_or("missing registry key")?
    };
    let record_key = registry_key
        .parse::<veilid_core::RecordKey>()
        .map_err(|e| format!("invalid registry key: {e}"))?;

    let local_oldest = load_local_oldest_lamports(state, community_id).await?;
    let channel_ids: Vec<String> = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .map(|community| {
                community
                    .channels
                    .iter()
                    .map(|channel| channel.id.clone())
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut peer_ads: Vec<(String, Vec<u8>, Vec<HistoryAd>)> = Vec::new();

    for subkey in 0..255u32 {
        let Some(value) = rc
            .get_dht_value(record_key.clone(), subkey, false)
            .await
            .map_err(|e| format!("registry read failed: {e}"))?
        else {
            continue;
        };
        if value.data().is_empty() {
            continue;
        }
        let Ok(presence) = serde_json::from_slice::<MemberPresence>(value.data()) else {
            continue;
        };
        if presence.route_blob.is_empty() || presence.history_ranges.is_empty() {
            continue;
        }
        let ads = presence
            .history_ranges
            .iter()
            .map(|range| HistoryAd {
                channel_id: hex::encode(range.channel_id.0),
                oldest_lamport: range.oldest_lamport,
                newest_lamport: range.newest_lamport,
            })
            .collect();
        peer_ads.push((
            hex::encode(presence.pseudonym_key.0),
            presence.route_blob,
            ads,
        ));
    }

    for channel_id in &channel_ids {
        let current_oldest = *local_oldest.get(channel_id).unwrap_or(&0);
        let needed_lamport = current_oldest.saturating_sub(1);
        let candidates: Vec<_> = peer_ads
            .iter()
            .enumerate()
            .filter_map(|(idx, (_peer, _route, ads))| {
                ads.iter()
                    .find(|ad| ad.channel_id == *channel_id)
                    .map(|ad| (idx, ad))
            })
            .collect();

        let selected = if needed_lamport == 0 {
            candidates
                .iter()
                .min_by_key(|(_, ad)| ad.oldest_lamport)
                .map(|(idx, _)| *idx)
        } else {
            select_best_peer(&candidates, needed_lamport)
        };

        let Some(selected_idx) = selected else {
            continue;
        };
        let Some((_, route_blob, ads)) = peer_ads.get(selected_idx) else {
            continue;
        };
        let Some(best_ad) = ads.iter().find(|ad| ad.channel_id == *channel_id) else {
            continue;
        };
        if best_ad.oldest_lamport >= current_oldest && current_oldest != 0 {
            continue;
        }

        let route_id = state_helpers::import_route_blob(state, route_blob)?;
        let request = CommunityEnvelope::Control(ControlPayload::SyncRequest {
            channel_id: channel_id.clone(),
            since_timestamp: 0,
        });
        let bytes =
            serde_json::to_vec(&request).map_err(|e| format!("sync request serialize: {e}"))?;
        let _ = rc
            .app_call(veilid_core::Target::RouteId(route_id), bytes)
            .await
            .map_err(|e| format!("sync request app_call failed: {e}"))?;
    }

    Ok(())
}

async fn load_local_oldest_lamports(
    state: &Arc<AppState>,
    community_id: &str,
) -> Result<HashMap<String, u64>, String> {
    use tauri::Manager as _;

    let app_handle = state_helpers::app_handle(state).ok_or("app handle unavailable")?;
    let pool: tauri::State<'_, crate::db::DbPool> = app_handle.state();
    let owner_key = state_helpers::current_owner_key(state)?;
    let cid = community_id.to_string();

    crate::db_helpers::db_call(pool.inner(), move |conn| {
        let mut stmt = conn.prepare(
            "SELECT conversation_id, COALESCE(MIN(lamport_ts), 0) \
             FROM messages \
             WHERE owner_key = ?1 AND community_id = ?2 AND conversation_type = 'channel' \
             GROUP BY conversation_id",
        )?;
        let rows = stmt.query_map(rusqlite::params![owner_key, cid], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
        })?;
        Ok(rows.filter_map(Result::ok).collect::<HashMap<_, _>>())
    })
    .await
}
