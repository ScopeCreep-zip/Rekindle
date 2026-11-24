use std::sync::Arc;

use crate::state::AppState;

use super::control_events::control_events_core;
use super::ingest_outcome::{IngestOutcome, ShellFollowUp};

/// A relayed Control the Host's community gate admitted (plan GI4): what
/// the Host ingests itself returns nothing, a `RequestMEK` a follow-up for
/// the `MekAdapter`, the rest `control_events_core`'s outcome.
pub(crate) async fn relayed_control_core(
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    payload: rekindle_codec::community::envelope::ControlPayload,
) -> IngestOutcome {
    use rekindle_codec::community::envelope::ControlPayload;

    match payload {
        // A member's own join notice or departure: the Host's one ingest, as
        // on the daemon (`gossip_ingest.rs`). The notice is a hint; the
        // Joined/Left a window shows and the departure rotation (ADR 0018)
        // follow the merged governance (`member_events.rs`), never the frame.
        payload @ (ControlPayload::MemberJoined { .. } | ControlPayload::MemberLeave { .. }) => {
            host_membership(state, community_id, sender_pseudonym, &payload).await;
            IngestOutcome::default()
        }
        payload @ (ControlPayload::ReactionAdded { .. }
        | ControlPayload::ReactionRemoved { .. }) => {
            // The Host's one ingest (plan GI3, the daemon's `ingest_gossip`
            // calls it too) gates, meters and journals the event.
            if let Ok(host) = crate::state_helpers::host_governance(state) {
                host.ingest_reaction(community_id, sender_pseudonym, 0, &payload);
            }
            IngestOutcome::default()
        }
        payload @ ControlPayload::RequestMEK { .. } => {
            IngestOutcome::follow_up(ShellFollowUp::MekRequest {
                community_id: community_id.to_owned(),
                payload,
            })
        }
        payload @ ControlPayload::RequestSegmentExpansion { .. } => {
            // The Host's one ingest (plan GI3, the daemon's `ingest_gossip`
            // calls it too) checks MANAGE_COMMUNITY and expands.
            if let Ok(host) = crate::state_helpers::host_governance(state) {
                host.on_segment_expansion_request(community_id, &payload)
                    .await;
            }
            IngestOutcome::default()
        }
        other => control_events_core(state, community_id, sender_pseudonym, other).await,
    }
}

pub(super) async fn handle_mek_request(
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
