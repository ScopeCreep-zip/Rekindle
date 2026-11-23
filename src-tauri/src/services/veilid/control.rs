use std::sync::Arc;

use crate::state::AppState;

use super::control_events::handle_control_events_and_threads;

pub(crate) async fn handle_relayed_control(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    payload: rekindle_codec::community::envelope::ControlPayload,
) {
    use rekindle_codec::community::envelope::ControlPayload;

    match payload {
        payload @ (ControlPayload::MemberJoined { .. } | ControlPayload::MemberLeave { .. }) => {
            host_membership(state, community_id, sender_pseudonym, &payload).await;
            super::control_membership::handle_membership_payload(
                app_handle,
                state,
                community_id,
                payload,
            );
        }
        payload @ (ControlPayload::ReactionAdded { .. }
        | ControlPayload::ReactionRemoved { .. }) => {
            // The Host's one ingest (plan GI3, the daemon's `ingest_gossip`
            // calls it too) gates, meters and journals the event.
            if let Ok(host) = crate::state_helpers::host_governance(state) {
                host.ingest_reaction(community_id, sender_pseudonym, 0, &payload);
            }
        }
        payload @ ControlPayload::RequestMEK { .. } => {
            handle_mek_request(app_handle, state, community_id, payload).await;
        }
        payload @ ControlPayload::RequestSegmentExpansion { .. } => {
            // The Host's one ingest (plan GI3, the daemon's `ingest_gossip`
            // calls it too) checks MANAGE_COMMUNITY and expands.
            if let Ok(host) = crate::state_helpers::host_governance(state) {
                host.on_segment_expansion_request(community_id, &payload)
                    .await;
            }
        }
        other => {
            handle_control_events_and_threads(
                app_handle,
                state,
                community_id,
                sender_pseudonym,
                other,
            )
            .await;
        }
    }
}

async fn handle_mek_request(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    payload: rekindle_codec::community::envelope::ControlPayload,
) {
    use rekindle_codec::community::envelope::ControlPayload;

    if let ControlPayload::RequestMEK {
        channel_id,
        needed_generation,
        requester_pseudonym,
        cascade_index,
    } = payload
    {
        {
            // The Host's responder (plan E3.2 S6) over the shell's key
            // adapter, until E3.5 moves the MEK adapters.
            let Some(host) = state.host.current() else {
                return;
            };
            let deps = match state.db.current() {
                Ok(pool) => crate::services::mek_adapter::MekAdapter::new(
                    Arc::clone(state),
                    app_handle.clone(),
                    pool,
                ),
                Err(error) => {
                    tracing::debug!(community = %community_id, %error, "RequestMEK ignored");
                    return;
                }
            };
            let request = rekindle_node::host::adapters::governance::MekRequest {
                channel_id,
                needed_generation,
                requester_pseudonym,
                cascade_index,
            };
            if let Err(error) = host
                .governance()
                .answer_mek_request(deps.as_ref(), community_id, &request)
                .await
            {
                tracing::debug!(community = %community_id, error = %error, "RequestMEK not answered");
            }
        }
    }
}

/// A membership control message (onboarding answers, a member's own join
/// notice or departure), handled by the Host's governance adapter on both hosts
/// (plan E3.1 B2e).
pub(super) async fn host_membership(
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    payload: &rekindle_codec::community::envelope::ControlPayload,
) {
    let Some(host) = state.host.current() else {
        tracing::debug!(community = %community_id, "membership control before the Host is up — dropped");
        return;
    };
    host.governance()
        .on_membership_control(community_id, sender_pseudonym, payload)
        .await;
}
