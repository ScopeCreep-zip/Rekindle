use std::sync::Arc;

// Voice signaling dispatch now goes through
// `crate::services::voice_signaling_adapter::handle_voice_signaling`.
use crate::state::AppState;

use super::control_sync::{
    check_gossip_moderation_permission, handle_sync_request, handle_sync_response,
};

pub(crate) fn handle_gossip_control_payloads(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    payload: rekindle_codec::community::envelope::ControlPayload,
) {
    use rekindle_codec::community::envelope::ControlPayload;

    match payload {
        payload @ (ControlPayload::AdminKeypairGrant { .. }
        | ControlPayload::SlotKeypairGrant { .. }) => {
            super::control::spawn_host_membership(state, community_id, sender_pseudonym, payload);
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
                    if let Err(e) =
                        crate::services::sync_communities::handle_community_record_change(
                            &state,
                            &db_pool,
                            &governance_key,
                        )
                        .await
                    {
                        tracing::debug!(error = %e, "governance change while locked — not rebuilt");
                    }
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
        | ControlPayload::KeyframeRequest { .. }
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
            handle_gossip_moderation(app_handle, state, community_id, sender_pseudonym, other);
        }
    }
}

fn handle_gossip_moderation(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    payload: rekindle_codec::community::envelope::ControlPayload,
) {
    use rekindle_codec::community::envelope::ControlPayload;

    if crate::state_helpers::current_owner_key(state).is_err() {
        tracing::debug!("gossip moderation: not logged in — dropped");
        return;
    }

    if !check_gossip_moderation_permission(state, community_id, sender_pseudonym, &payload) {
        return;
    }

    match payload {
        ControlPayload::Kick { target_pseudonym } => {
            remove_member_from_local_state(
                app_handle,
                state,
                community_id,
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
                community_id,
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
    community_id: &str,
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
    match state.host.current() {
        Some(host) => host
            .governance()
            .forget_member(community_id, &target_pseudonym),
        None => {
            tracing::debug!(community = %community_id, "{label}: the Host is not up, the roster row stays");
        }
    }
    crate::event_dispatch::emit_membership(
        app_handle,
        rekindle_types::subscription_events::MembershipEvent::Removed {
            community: community_id.to_string(),
            pseudonym: target_pseudonym,
        },
    );
}
