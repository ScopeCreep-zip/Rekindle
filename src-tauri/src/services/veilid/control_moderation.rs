use std::sync::Arc;

// Voice signaling dispatch now goes through
// `crate::services::voice_signaling_adapter::handle_voice_signaling`.
use crate::state::AppState;

pub(crate) fn handle_gossip_control_payloads(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    payload: rekindle_codec::community::envelope::ControlPayload,
) {
    use rekindle_codec::community::envelope::ControlPayload;

    match payload {
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
            handle_gossip_moderation(state, community_id, sender_pseudonym, other);
        }
    }
}

fn handle_gossip_moderation(
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    payload: rekindle_codec::community::envelope::ControlPayload,
) {
    use rekindle_codec::community::envelope::ControlPayload;

    match payload {
        // A relayed kick is the Host's, the same on both hosts (GI3): it
        // checks the kicker's authority under the merged governance, forgets
        // the member and starts the departure rotation; the membership event
        // is the merge's. The gossip dispatcher stays sync.
        ControlPayload::Kick { target_pseudonym } => {
            let Some(host) = state.host.current() else {
                tracing::debug!(community = %community_id, "relayed kick: the Host is not up — dropped");
                return;
            };
            let (community, kicker) = (community_id.to_owned(), sender_pseudonym.to_owned());
            crate::state_helpers::login_scope_or_closed(state).spawn_or_drop(
                "relayed kick",
                async move {
                    host.governance()
                        .on_relayed_kick(&community, &target_pseudonym, &kicker)
                        .await;
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
