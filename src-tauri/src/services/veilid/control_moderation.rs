use std::sync::Arc;

use crate::db_helpers::db_fire;
use rekindle_db::Db;
// Voice signaling dispatch now goes through
// `crate::services::voice_signaling_adapter::handle_voice_signaling`.
use crate::state::AppState;

use super::control_sync::{
    check_gossip_moderation_permission, handle_sync_request, handle_sync_response,
};
use crate::services::governance_adapter;

pub(crate) fn handle_gossip_control_payloads(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    payload: rekindle_codec::community::envelope::ControlPayload,
) {
    use rekindle_codec::community::envelope::ControlPayload;

    match payload {
        ControlPayload::AdminKeypairGrant {
            wrapped_owner_keypair,
            wrapped_slot_seed,
        } => {
            governance_adapter::process_admin_keypair_grant(
                state,
                app_handle,
                community_id,
                sender_pseudonym,
                &wrapped_owner_keypair,
                &wrapped_slot_seed,
            );
        }
        ControlPayload::SlotKeypairGrant {
            slot_index,
            segment_index,
            wrapped_slot_keypair,
        } => {
            governance_adapter::process_slot_keypair_grant(
                state,
                app_handle,
                community_id,
                sender_pseudonym,
                slot_index,
                segment_index,
                &wrapped_slot_keypair,
            );
        }
        ControlPayload::SyncRequest {
            channel_id,
            since_timestamp,
        } => {
            handle_sync_request(
                state,
                community_id,
                sender_pseudonym,
                &channel_id,
                since_timestamp,
            );
        }
        ControlPayload::SyncResponse {
            channel_id,
            messages,
        } => {
            {
                let mut communities = state.communities.write();
                if let Some(cs) = communities.get_mut(community_id) {
                    cs.pending_syncs.remove(&channel_id);
                }
            }
            handle_sync_response(app_handle, state, community_id, &channel_id, &messages);
        }
        ControlPayload::GovernanceUpdated {
            governance_key,
            subkey_index: _,
            lamport_ts: _,
        } => {
            let Ok(pool) = state.db.current() else {
                tracing::debug!("gossip moderation: no identity database — dropped");
                return;
            };
            let db_pool = pool.clone();
            let state = Arc::clone(state);
            crate::state_helpers::login_scope_or_closed(&state).spawn_or_drop(
                "community record change",
                async move {
                    let _ = crate::services::sync_communities::handle_community_record_change(
                        &state,
                        &db_pool,
                        &governance_key,
                    )
                    .await;
                },
            );
        }
        payload if rekindle_voice::signaling::is_voice_signaling(&payload) => {
            // Voice signaling adapter handles the spawn-and-forget
            // dispatch internally — the gossip dispatcher stays sync.
            crate::services::voice_signaling_adapter::handle_voice_signaling(
                app_handle,
                state,
                community_id,
                sender_pseudonym,
                payload,
            );
        }
        ControlPayload::VideoFragment { .. }
        | ControlPayload::VideoParityFragment { .. }
        | ControlPayload::FrameAck { .. }
        | ControlPayload::KeyframeRequest { .. }
        | ControlPayload::BandwidthEstimate { .. }
        | ControlPayload::TopologyChange { .. }
        | ControlPayload::MediaCapabilities { .. } => {
            crate::services::community::video::handle_video_payload(
                app_handle,
                state,
                community_id,
                sender_pseudonym,
                payload,
            );
        }
        ControlPayload::LinkPreview {
            channel_id,
            message_id,
            url,
            title,
            description,
            site_name,
            fetched_at,
        } => {
            crate::services::community::link_previews::handle_incoming_link_preview(
                app_handle,
                state,
                community_id,
                sender_pseudonym,
                channel_id,
                rekindle_types::link_preview::LinkPreview {
                    message_id,
                    url,
                    title,
                    description,
                    site_name,
                    fetched_at,
                },
            );
        }
        other => {
            let Ok(pool) = state.db.current() else {
                tracing::debug!("gossip moderation: no identity database — dropped");
                return;
            };
            handle_gossip_moderation(
                app_handle,
                state,
                &pool,
                community_id,
                sender_pseudonym,
                other,
            );
        }
    }
}

fn handle_gossip_moderation(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &Db,
    community_id: &str,
    sender_pseudonym: &str,
    payload: rekindle_codec::community::envelope::ControlPayload,
) {
    use rekindle_codec::community::envelope::ControlPayload;

    let Ok(owner_key) = crate::state_helpers::current_owner_key(state) else {
        return;
    };

    if !check_gossip_moderation_permission(state, community_id, sender_pseudonym, &payload) {
        return;
    }

    match payload {
        ControlPayload::Kick { target_pseudonym } => {
            remove_member_from_local_state(
                app_handle,
                state,
                pool,
                community_id,
                owner_key,
                target_pseudonym.clone(),
                "kick_member_remove",
            );
            // A kicked member still holds the current keys (plan D20).
            crate::services::community::spawn_departure_rotations(
                app_handle,
                state,
                community_id,
                &target_pseudonym,
            );
        }
        ControlPayload::Ban {
            target_pseudonym, ..
        } => {
            remove_member_from_local_state(
                app_handle,
                state,
                pool,
                community_id,
                owner_key,
                target_pseudonym.clone(),
                "ban_member_remove",
            );
            crate::services::community::spawn_departure_rotations(
                app_handle,
                state,
                community_id,
                &target_pseudonym,
            );
        }
        ControlPayload::Unban { .. } => {}
        ControlPayload::TimeoutMember {
            target_pseudonym,
            duration_seconds,
            ..
        } => {
            let timeout_until = rekindle_utils::timestamp_secs() + duration_seconds;
            let ok = owner_key.clone();
            let cid = community_id.to_string();
            let tp = target_pseudonym.clone();
            db_fire(pool, "timeout_member", move |conn| {
                rekindle_db::repo::members::set_timeout(conn, &ok, &cid, &tp, Some(timeout_until))
            });
            crate::event_dispatch::emit_membership(
                app_handle,
                rekindle_types::subscription_events::MembershipEvent::TimeoutStatusChanged {
                    community: community_id.to_string(),
                    pseudonym: target_pseudonym,
                    timeout_until: Some(timeout_until),
                },
            );
        }
        ControlPayload::RemoveTimeout { target_pseudonym } => {
            let ok = owner_key.clone();
            let cid = community_id.to_string();
            let tp = target_pseudonym.clone();
            db_fire(pool, "remove_timeout", move |conn| {
                rekindle_db::repo::members::set_timeout(conn, &ok, &cid, &tp, None)
            });
            crate::event_dispatch::emit_membership(
                app_handle,
                rekindle_types::subscription_events::MembershipEvent::TimeoutStatusChanged {
                    community: community_id.to_string(),
                    pseudonym: target_pseudonym,
                    timeout_until: None,
                },
            );
        }
        other => {
            tracing::trace!(
                community = %community_id,
                payload = ?other,
                "received unhandled moderation/structural payload"
            );
        }
    }
}

fn remove_member_from_local_state(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &Db,
    community_id: &str,
    owner_key: String,
    target_pseudonym: String,
    label: &'static str,
) {
    let my_pseudonym = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .and_then(|cs| cs.my_pseudonym_key.clone())
    };

    if my_pseudonym.as_deref() == Some(&target_pseudonym) {
        crate::event_dispatch::emit_subscription(
            app_handle,
            &rekindle_types::subscription_events::SubscriptionEvent::System(
                rekindle_types::subscription_events::SystemEvent::Kicked {
                    community: community_id.to_string(),
                },
            ),
        );
        return;
    }

    {
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(community_id) {
            cs.known_members.remove(&target_pseudonym);
            if let Some(ref mut gossip) = cs.gossip {
                gossip.online_members.remove(&target_pseudonym);
                gossip.peers.remove(&target_pseudonym);
            }
        }
    }
    crate::services::community::analytics::log_member_leave(
        pool,
        &owner_key,
        community_id,
        &target_pseudonym,
    );
    let cid = community_id.to_string();
    let tp = target_pseudonym.clone();
    db_fire(pool, label, move |conn| {
        rekindle_db::repo::members::delete(conn, &owner_key, &cid, &tp)
    });
    crate::event_dispatch::emit_membership(
        app_handle,
        rekindle_types::subscription_events::MembershipEvent::Removed {
            community: community_id.to_string(),
            pseudonym: target_pseudonym,
        },
    );
}
