use std::sync::Arc;

use tauri::Manager;

use crate::state::AppState;
use crate::state_helpers;

use super::current_presence_status;

pub(super) async fn run_initial_sync(state: &Arc<AppState>, community_id: &str, d: usize) {
    if d > 0 {
        let (my_pk, our_route) = {
            let communities = state.communities.read();
            let cs = communities.get(community_id);
            (
                cs.and_then(|c| c.my_pseudonym_key.clone())
                    .unwrap_or_default(),
                state_helpers::our_route_blob(state),
            )
        };
        if our_route.is_some() {
            let presence_envelope =
                rekindle_protocol::dht::community::envelope::CommunityEnvelope::PresenceUpdate {
                    pseudonym_key: my_pk,
                    status: current_presence_status(state).to_string(),
                    game_info: None,
                    route_blob: our_route,
                };
            let _ =
                crate::services::community::send_to_mesh(state, community_id, &presence_envelope);
        } else {
            tracing::warn!(
                community = %community_id,
                "skipping PresenceUpdate broadcast — route_blob not yet available"
            );
            return;
        }
    }

    let all_channel_ids: Vec<String> = {
        let communities = state.communities.read();
        communities
            .get(community_id)
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
            })
            .await
            .unwrap_or(0);

            let sync_req = rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
                rekindle_protocol::dht::community::envelope::ControlPayload::SyncRequest {
                    channel_id: ch_id.clone(),
                    since_timestamp: last_ts.cast_unsigned(),
                },
            );
            let _ = crate::services::community::send_to_mesh(state, community_id, &sync_req);
            let now = rekindle_utils::timestamp_secs();
            let mut communities = state.communities.write();
            if let Some(cs) = communities.get_mut(community_id) {
                cs.pending_syncs.insert(ch_id.clone(), (now, 1));
            }
        }
    }

    let channel_entries: Vec<(String, String)> = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .map(|cs| {
                cs.channel_log_keys
                    .iter()
                    .map(|(ch_id, record_key)| (ch_id.clone(), record_key.clone()))
                    .collect()
            })
            .unwrap_or_default()
    };
    let member_count = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .map_or(0, |cs| u32::try_from(cs.known_members.len()).unwrap_or(255))
    };

    if !channel_entries.is_empty() && member_count > 0 {
        if let Some(rc) = state_helpers::safe_routing_context(state) {
            for (ch_id, record_key) in &channel_entries {
                match rekindle_protocol::dht::community::channel_record::read_all_channel_messages(
                    &rc,
                    record_key,
                    member_count,
                )
                .await
                {
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
                            crate::db_helpers::db_fire(
                                pool.inner(),
                                "smpl_channel_catchup",
                                move |conn| {
                                    for msg in &messages {
                                        let mid = msg.message_id.as_deref().unwrap_or("");
                                        if mid.is_empty() {
                                            continue;
                                        }
                                        let exists: bool = conn.query_row(
                                            "SELECT EXISTS(SELECT 1 FROM messages WHERE owner_key=?1 AND message_id=?2)",
                                            rusqlite::params![ok, mid],
                                            |r| r.get(0),
                                        ).unwrap_or(false);
                                        if exists {
                                            continue;
                                        }
                                        let _ = conn.execute(
                                            "INSERT OR IGNORE INTO messages \
                                             (owner_key, conversation_id, conversation_type, sender_key, body, timestamp, message_id, lamport_ts) \
                                             VALUES (?1, ?2, 'channel', ?3, ?4, ?5, ?6, ?7)",
                                            rusqlite::params![
                                                ok,
                                                channel,
                                                msg.sender_pseudonym,
                                                String::from_utf8_lossy(&msg.ciphertext),
                                                msg.timestamp,
                                                mid,
                                                msg.lamport_ts,
                                            ],
                                        );
                                    }
                                    Ok(())
                                },
                            );
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

    let mut communities = state.communities.write();
    if let Some(cs) = communities.get_mut(community_id) {
        if let Some(ref mut g) = cs.gossip {
            g.needs_initial_sync = false;
        }
    }
    tracing::info!(community = %community_id, "initial sync complete");
}
