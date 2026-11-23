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
        // A member's own join notice or departure: the Host's one ingest, as
        // on the daemon (`gossip_ingest.rs`). The notice is a hint; the
        // Joined/Left a window shows and the departure rotation (ADR 0018)
        // follow the merged governance (`member_events.rs`), never the frame.
        payload @ (ControlPayload::MemberJoined { .. } | ControlPayload::MemberLeave { .. }) => {
            host_membership(state, community_id, sender_pseudonym, &payload).await;
        }
        payload @ (ControlPayload::ReactionAdded { .. }
        | ControlPayload::ReactionRemoved { .. }) => {
            handle_channel_event_payload(app_handle, community_id, payload);
        }
        payload @ (ControlPayload::RequestMEK { .. }
        | ControlPayload::RequestSegmentExpansion { .. }) => {
            handle_mek_and_segment_requests(app_handle, state, community_id, payload).await;
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

fn handle_channel_event_payload(
    app_handle: &tauri::AppHandle,
    community_id: &str,
    payload: rekindle_codec::community::envelope::ControlPayload,
) {
    use rekindle_codec::community::envelope::ControlPayload;

    match payload {
        ControlPayload::ReactionAdded {
            channel_id,
            message_id,
            emoji,
            reactor_pseudonym,
        } => {
            crate::event_dispatch::emit_subscription(
                app_handle,
                &rekindle_types::subscription_events::SubscriptionEvent::Social(
                    rekindle_types::subscription_events::SocialEvent::ReactionAdded {
                        community: community_id.to_string(),
                        channel: channel_id,
                        message_id,
                        emoji,
                        reactor_pseudonym,
                    },
                ),
            );
        }
        ControlPayload::ReactionRemoved {
            channel_id,
            message_id,
            emoji,
            reactor_pseudonym,
        } => {
            crate::event_dispatch::emit_subscription(
                app_handle,
                &rekindle_types::subscription_events::SubscriptionEvent::Social(
                    rekindle_types::subscription_events::SocialEvent::ReactionRemoved {
                        community: community_id.to_string(),
                        channel: channel_id,
                        message_id,
                        emoji,
                        reactor_pseudonym,
                    },
                ),
            );
        }
        _ => {}
    }
}

async fn handle_mek_and_segment_requests(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    payload: rekindle_codec::community::envelope::ControlPayload,
) {
    use rekindle_codec::community::envelope::ControlPayload;

    match payload {
        ControlPayload::RequestMEK {
            channel_id,
            needed_generation,
            requester_pseudonym,
            cascade_index,
        } => {
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
        ControlPayload::RequestSegmentExpansion {
            community_id: req_community_id,
            requester_pseudonym,
            full_segment_index,
        } => {
            // P4.3 — admin handler. Any peer with MANAGE_COMMUNITY who
            // sees this gossip request expands the community by one
            // segment. The first admin's `expand_community_segment`
            // call wins via CRDT — later admins see `next_segment_index
            // > full_segment_index + 1` in their merged state and the
            // pre-flight check inside `expand_community_segment`
            // (`highest_segment_full`) returns false, no-op'ing the
            // expansion.
            //
            // Defense-in-depth: ignore the request if the community_id
            // doesn't match the gossip envelope's community (a forged
            // envelope can't redirect us).
            if req_community_id != community_id {
                tracing::debug!(
                    envelope_cid = %community_id,
                    req_cid = %req_community_id,
                    "RequestSegmentExpansion ignored: community_id mismatch"
                );
                return;
            }
            // Permission check: only act if we have MANAGE_COMMUNITY.
            // Other peers without the bit ignore — that's the spec's
            // "any admin reacts" model.
            let have_manage_community = rekindle_governance::permissions::has_capability(
                crate::state_helpers::my_permissions(state, community_id, None),
                rekindle_types::permissions::MANAGE_COMMUNITY,
            );
            if !have_manage_community {
                tracing::debug!(
                    community = %community_id,
                    requester = %requester_pseudonym,
                    full_segment_index,
                    "RequestSegmentExpansion ignored: we lack MANAGE_COMMUNITY"
                );
                return;
            }
            tracing::info!(
                community = %community_id,
                requester = %requester_pseudonym,
                full_segment_index,
                "RequestSegmentExpansion received — calling expand_community_segment"
            );
            if let Err(error) =
                crate::services::community::segments::expand_community_segment(state, community_id)
                    .await
            {
                // The "current segment still has open slots" + "segment
                // cap reached" errors are expected race-conditions when
                // multiple admins receive the same request. Log at
                // debug level to avoid alarm-fatigue.
                tracing::debug!(
                    community = %community_id,
                    error = %error,
                    "expand_community_segment did not run (expected on concurrent admin races)"
                );
            }
        }
        _ => {}
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
