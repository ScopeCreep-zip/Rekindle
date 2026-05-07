use std::sync::Arc;

use crate::db::DbPool;
use crate::db_helpers::db_fire;
use crate::state::AppState;
use crate::state_helpers;
use tauri::{Emitter, Manager};

use super::control_events::handle_control_events_and_threads;
use crate::services::veilid::legacy::membership::{
    handle_join_accepted, handle_member_roles_changed, join_accepted_data,
};
use crate::services::veilid::legacy::onboarding::handle_peer_assisted_join;

pub(crate) async fn handle_relayed_control(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    community_id: &str,
    sender_pseudonym: &str,
    payload: rekindle_protocol::dht::community::envelope::ControlPayload,
) {
    use rekindle_protocol::dht::community::envelope::ControlPayload;

    match payload {
        payload @ (ControlPayload::MemberJoinRequest { .. }
        | ControlPayload::MemberJoined { .. }
        | ControlPayload::MemberRemoved { .. }
        | ControlPayload::MemberLeave { .. }
        | ControlPayload::MemberTimedOut { .. }) => {
            handle_membership_payload(app_handle, state, pool, community_id, payload);
        }
        payload @ (ControlPayload::MessageEdited { .. }
        | ControlPayload::MessageDeleted { .. }
        | ControlPayload::ReactionAdded { .. }
        | ControlPayload::ReactionRemoved { .. }) => {
            handle_channel_event_payload(app_handle, state, community_id, payload);
        }
        payload @ (ControlPayload::MemberRolesChanged { .. }
        | ControlPayload::JoinAccepted { .. }
        | ControlPayload::JoinRejected { .. }
        | ControlPayload::RequestMEK { .. }) => {
            handle_join_and_roles_payload(
                app_handle,
                state,
                community_id,
                sender_pseudonym,
                payload,
            )
            .await;
        }
        other => {
            handle_control_events_and_threads(
                app_handle,
                state,
                pool,
                community_id,
                sender_pseudonym,
                other,
            )
            .await;
        }
    }
}

fn handle_membership_payload(
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

            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::MemberJoined {
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
                    crate::services::community::raid_detection::observe_join(
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
                let _ = app_handle.emit(
                    "community-event",
                    CommunityEvent::RaidDetected {
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

            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::MemberRemoved {
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
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::MemberTimedOut {
                    community_id: community_id.to_string(),
                    pseudonym_key,
                    timeout_until,
                },
            );
        }
        _ => {}
    }
}

fn decrypt_edited_message_body(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    new_ciphertext: &[u8],
) -> String {
    let decrypted = {
        let mek_cache = state.channel_mek_cache.lock();
        mek_cache
            .get(&(community_id.to_string(), channel_id.to_string()))
            .map(|mek| mek.decrypt(new_ciphertext))
    };
    match decrypted {
        Some(Ok(bytes)) => String::from_utf8_lossy(&bytes).into_owned(),
        Some(Err(_)) => "(decryption failed)".to_string(),
        None => {
            let mek_cache = state.mek_cache.lock();
            if let Some(mek) = mek_cache.get(community_id) {
                match mek.decrypt(new_ciphertext) {
                    Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                    Err(_) => "(decryption failed)".to_string(),
                }
            } else {
                "(no MEK available)".to_string()
            }
        }
    }
}

fn handle_channel_event_payload(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    payload: rekindle_protocol::dht::community::envelope::ControlPayload,
) {
    use crate::channels::CommunityEvent;
    use rekindle_protocol::dht::community::envelope::ControlPayload;

    match payload {
        ControlPayload::MessageEdited {
            channel_id,
            message_id,
            new_ciphertext,
            mek_generation: _,
            edited_at,
        } => {
            let new_body =
                decrypt_edited_message_body(state, community_id, &channel_id, &new_ciphertext);
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::MessageEdited {
                    community_id: community_id.to_string(),
                    channel_id,
                    message_id,
                    new_body,
                    edited_at,
                },
            );
        }
        ControlPayload::MessageDeleted {
            channel_id,
            message_id,
        } => {
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::MessageDeleted {
                    community_id: community_id.to_string(),
                    channel_id,
                    message_id,
                },
            );
        }
        ControlPayload::ReactionAdded {
            channel_id,
            message_id,
            emoji,
            reactor_pseudonym,
        } => {
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::ReactionAdded {
                    community_id: community_id.to_string(),
                    channel_id,
                    message_id,
                    emoji,
                    reactor_pseudonym,
                },
            );
        }
        ControlPayload::ReactionRemoved {
            channel_id,
            message_id,
            emoji,
            reactor_pseudonym,
        } => {
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::ReactionRemoved {
                    community_id: community_id.to_string(),
                    channel_id,
                    message_id,
                    emoji,
                    reactor_pseudonym,
                },
            );
        }
        _ => {}
    }
}

async fn handle_join_and_roles_payload(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    payload: rekindle_protocol::dht::community::envelope::ControlPayload,
) {
    use crate::channels::CommunityEvent;
    use rekindle_protocol::dht::community::envelope::ControlPayload;

    match payload {
        ControlPayload::MemberRolesChanged {
            pseudonym_key,
            role_ids,
        } => {
            handle_member_roles_changed(app_handle, state, community_id, &pseudonym_key, &role_ids);
        }
        ControlPayload::JoinAccepted {
            mek_encrypted,
            mek_generation,
            members,
            member_registry_key,
            slot_index,
            wrapped_slot_seed,
        } => {
            handle_join_accepted(
                app_handle,
                state,
                community_id,
                sender_pseudonym,
                join_accepted_data(
                    &mek_encrypted,
                    mek_generation,
                    &members,
                    member_registry_key.as_deref(),
                    slot_index,
                    wrapped_slot_seed.as_deref(),
                ),
            )
            .await;
        }
        ControlPayload::JoinRejected { reason } => {
            tracing::warn!(community = %community_id, reason = %reason, "join request rejected by peer");
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::JoinRejected {
                    community_id: community_id.to_string(),
                    reason,
                },
            );
        }
        ControlPayload::RequestMEK {
            channel_id,
            needed_generation,
            requester_pseudonym,
            cascade_index,
        } => {
            if let Err(error) = crate::services::community::handle_request_mek(
                app_handle,
                state,
                community_id,
                &channel_id,
                needed_generation,
                &requester_pseudonym,
                cascade_index,
            )
            .await
            {
                tracing::debug!(community = %community_id, error = %error, "RequestMEK ignored");
            }
        }
        _ => {}
    }
}
