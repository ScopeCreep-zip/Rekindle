//! Phase 23.D.4 — `handle_membership_payload` extracted from
//! `services/veilid/control.rs` to keep that file under the 500-LoC
//! cap (Invariant 1). Dispatches the five Member* ControlPayload
//! variants — MemberJoinRequest / MemberJoined / MemberRemoved /
//! MemberLeave / MemberTimedOut — to their per-variant handlers.

use std::sync::Arc;

use tauri::Manager;

use crate::db::DbPool;
use crate::db_helpers::db_fire;
use crate::services::veilid::legacy::onboarding::handle_peer_assisted_join;
use crate::state::AppState;
use crate::state_helpers;

pub(super) fn handle_membership_payload(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    community_id: &str,
    payload: rekindle_protocol::dht::community::envelope::ControlPayload,
) {
    use crate::channels::CommunityEvent;
    use rekindle_protocol::dht::community::envelope::ControlPayload;

    match payload {
        ControlPayload::MemberJoinRequest {
            pseudonym_key,
            display_name,
            claimed_subkey_index,
            route_blob,
            invite_code,
            ..
        } => {
            handle_peer_assisted_join(
                app_handle,
                state,
                pool,
                community_id,
                &pseudonym_key,
                &display_name,
                claimed_subkey_index,
                route_blob.as_deref(),
                invite_code.as_deref(),
            );
        }
        ControlPayload::MemberJoined {
            pseudonym_key,
            display_name,
            role_ids,
            status,
            route_blob,
        } => {
            {
                let mut communities = state.communities.write();
                if let Some(cs) = communities.get_mut(community_id) {
                    cs.known_members.insert(pseudonym_key.clone());
                }
            }

            if status != "offline" {
                if let Some(ref blob) = route_blob {
                    if !blob.is_empty() {
                        let mut communities = state.communities.write();
                        if let Some(cs) = communities.get_mut(community_id) {
                            if cs.gossip.is_none() {
                                cs.gossip = Some(crate::state::GossipOverlay::default());
                            }
                            if let Some(ref mut gossip) = cs.gossip {
                                let member = crate::state::OnlineMember {
                                    route_blob: blob.clone(),
                                    status: status.clone(),
                                    last_seen: rekindle_utils::timestamp_secs(),
                                };
                                gossip
                                    .online_members
                                    .insert(pseudonym_key.clone(), member.clone());
                                gossip.peers.insert(pseudonym_key.clone(), member);
                            }
                        }
                    }
                }
            }
            let pool: tauri::State<'_, DbPool> = app_handle.state();
            let owner_key = state_helpers::current_owner_key(state).unwrap_or_default();
            let cid = community_id.to_string();
            let pk = pseudonym_key.clone();
            let dn = display_name.clone();
            let rids = role_ids.clone();
            crate::db_helpers::db_fire(pool.inner(), "persist MemberJoined", move |conn| {
                let role_ids_json = serde_json::to_string(&rids).unwrap_or_else(|_| "[0,1]".into());
                let now = crate::db::timestamp_now();
                conn.execute(
                    "INSERT OR IGNORE INTO community_members \
                     (owner_key, community_id, pseudonym_key, display_name, role_ids, joined_at) \
                     VALUES (?, ?, ?, ?, ?, ?)",
                    rusqlite::params![owner_key, cid, pk, dn, role_ids_json, now],
                )?;
                Ok(())
            });

            crate::event_dispatch::emit_live(
                app_handle,
                "community-event",
                &CommunityEvent::MemberJoined {
                    community_id: community_id.to_string(),
                    pseudonym_key: pseudonym_key.clone(),
                    display_name,
                    role_ids,
                },
            );

            // Architecture §20.6 — record the join in the per-community
            // sliding window; emit a raid alert if the rate trips the
            // policy threshold.
            let alert = {
                let mut communities = state.communities.write();
                if let Some(cs) = communities.get_mut(community_id) {
                    let policy = cs
                        .governance_state
                        .as_ref()
                        .and_then(|gs| gs.community_policy.as_ref())
                        .cloned();
                    rekindle_governance::raid_detection::observe_join(
                        &mut cs.recent_member_joins,
                        rekindle_utils::timestamp_secs(),
                        &pseudonym_key,
                        policy.as_ref(),
                    )
                } else {
                    None
                }
            };
            if let Some(alert) = alert {
                crate::event_dispatch::emit_live(
                    app_handle,
                    "community-event",
                    &CommunityEvent::RaidDetected {
                        community_id: community_id.to_string(),
                        joins_in_window: alert.joins_in_window,
                        max_joins_per_interval: alert.max_joins_per_interval,
                        join_interval_seconds: alert.join_interval_seconds,
                    },
                );
                tracing::warn!(
                    community = %community_id,
                    joins = alert.joins_in_window,
                    threshold = alert.max_joins_per_interval,
                    interval_s = alert.join_interval_seconds,
                    "raid threshold exceeded — alerting moderators (architecture §20.6)"
                );
            }
        }
        ControlPayload::MemberRemoved { pseudonym_key }
        | ControlPayload::MemberLeave { pseudonym_key } => {
            let departed_pseudonym = pseudonym_key.clone();
            let pool: tauri::State<'_, DbPool> = app_handle.state();
            let owner_key = state_helpers::current_owner_key(state).unwrap_or_default();
            crate::services::community::analytics::log_member_leave(
                pool.inner(),
                &owner_key,
                community_id,
                &pseudonym_key,
            );
            let cid = community_id.to_string();
            let pk = pseudonym_key.clone();
            crate::db_helpers::db_fire(pool.inner(), "persist MemberRemoved/Leave", move |conn| {
                conn.execute(
                    "DELETE FROM community_members WHERE owner_key = ? AND community_id = ? AND pseudonym_key = ?",
                    rusqlite::params![owner_key, cid, pk],
                )?;
                Ok(())
            });

            {
                let mut communities = state.communities.write();
                if let Some(cs) = communities.get_mut(community_id) {
                    cs.known_members.remove(&pseudonym_key);
                    if let Some(ref mut gossip) = cs.gossip {
                        gossip.online_members.remove(&pseudonym_key);
                        gossip.peers.remove(&pseudonym_key);
                    }
                }
            }

            crate::event_dispatch::emit_live(
                app_handle,
                "community-event",
                &CommunityEvent::MemberRemoved {
                    community_id: community_id.to_string(),
                    pseudonym_key,
                },
            );

            let state_clone = state.clone();
            let app_handle = app_handle.clone();
            let community_id = community_id.to_string();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = crate::services::community::rotate_text_mek_for_departure(
                    &app_handle,
                    &state_clone,
                    &community_id,
                    &departed_pseudonym,
                )
                .await
                {
                    tracing::debug!(community = %community_id, error = %error, "text MEK rotation skipped after departure");
                }
            });
        }
        ControlPayload::MemberTimedOut {
            pseudonym_key,
            timeout_until,
        } => {
            let ok = state_helpers::current_owner_key(state).unwrap_or_default();
            let cid = community_id.to_string();
            let tp = pseudonym_key.clone();
            db_fire(pool, "relayed_member_timed_out", move |conn| {
                conn.execute(
                    "UPDATE community_members SET timeout_until = ?1 \
                     WHERE owner_key = ?2 AND community_id = ?3 AND pseudonym_key = ?4",
                    rusqlite::params![timeout_until, ok, cid, tp],
                )?;
                Ok(())
            });
            crate::event_dispatch::emit_live(
                app_handle,
                "community-event",
                &CommunityEvent::MemberTimedOut {
                    community_id: community_id.to_string(),
                    pseudonym_key,
                    timeout_until,
                },
            );
        }
        _ => {}
    }
}
