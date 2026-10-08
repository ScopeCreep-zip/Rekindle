use std::collections::HashMap;
use std::sync::Arc;

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_sync::history::{select_best_peer, select_deepest_peer, HistoryAd};
use rekindle_types::presence::MemberPresence;

use crate::state::AppState;
use crate::state_helpers;

pub fn schedule_history_catchup(state: Arc<AppState>, community_id: String) {
    let scope = state_helpers::community_scope(&state, &community_id);
    scope.spawn_with_token_or_drop("community history catch-up", |stop| async move {
        let settle = tokio::time::sleep(std::time::Duration::from_secs(15));
        if stop.run_until_cancelled(settle).await.is_none() {
            return;
        }
        if let Err(error) = request_history_catchup(&state, &community_id, &stop).await {
            tracing::debug!(community = %community_id, error = %error, "history ad catchup skipped");
        }
    });
}

/// Reads the registry slot by slot, stopping before the next read once
/// `stop` is cancelled.
async fn request_history_catchup(
    state: &Arc<AppState>,
    community_id: &str,
    stop: &tokio_util::sync::CancellationToken,
) -> Result<(), String> {
    let pool = state_helpers::record_pool(state)?;
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

    // The registry is the membership index: one inspect over its range,
    // then a read of each row that holds a value (plan C7.12), not 255 reads
    // one after another.
    if stop.is_cancelled() {
        return Ok(());
    }
    let all_slots: Vec<u32> = (0..rekindle_records::schema::MAX_MEMBERS_PER_SEGMENT)
        .filter_map(|slot| u32::try_from(slot).ok())
        .collect();
    let lease = pool
        .acquire(&record_key, None)
        .await
        .map_err(|e| format!("registry open failed: {e}"))?;
    let rows = pool.read_changed(lease, &all_slots).await;
    pool.release(lease).await;
    let rows = rows.map_err(|e| format!("registry read failed: {e}"))?;
    for (_, value) in rows {
        if value.data().is_empty() {
            continue;
        }
        let Ok(presence) = serde_json::from_slice::<MemberPresence>(value.data()) else {
            continue;
        };
        // Architecture §26 W26 — verify before trusting history_ranges,
        // otherwise a forger could advertise bogus ranges to mislead the
        // mutual-aid sync.
        let Ok(sig_arr): Result<[u8; 64], _> = presence.signature.as_slice().try_into() else {
            continue;
        };
        if rekindle_secrets::derive::verify_pseudonym_signature(
            &presence.pseudonym_key.0,
            &presence.signing_bytes(),
            &sig_arr,
        )
        .is_err()
        {
            continue;
        }
        if presence.route_blob.is_empty() {
            continue;
        }
        // W11.2 — history_ranges are encrypted under the community MEK.
        // Members without the matching MEK generation cached (fresh
        // joiners pre-distribution, or after a rotation we missed) skip
        // this peer's advertisement and fall through to direct DHT
        // reads as before.
        let Some(encrypted) = presence.history_ranges_encrypted.as_ref() else {
            continue;
        };
        let Some(decrypted) = super::super::presence::registry::decrypt_history_ranges(
            state,
            community_id,
            encrypted,
        ) else {
            continue;
        };
        if decrypted.is_empty() {
            continue;
        }
        let ads = decrypted
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
        // Each request is an `app_call`: stop before the next one (C4.L1).
        if stop.is_cancelled() {
            return Ok(());
        }
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
            select_deepest_peer(&candidates)
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

        let request = CommunityEnvelope::Control(ControlPayload::SyncRequest {
            channel_id: channel_id.clone(),
            since_timestamp: 0,
        });
        let bytes =
            serde_json::to_vec(&request).map_err(|e| format!("sync request serialize: {e}"))?;
        state_helpers::call_route_blob(state, route_blob, bytes)
            .await
            .map_err(|e| format!("sync request failed: {e}"))?;
    }

    Ok(())
}

async fn load_local_oldest_lamports(
    state: &Arc<AppState>,
    community_id: &str,
) -> Result<HashMap<String, u64>, String> {
    let pool = state.db.current()?;
    let owner_key = state_helpers::current_owner_key(state)?;
    let cid = community_id.to_string();

    crate::db_helpers::db_call(&pool, move |conn| {
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
