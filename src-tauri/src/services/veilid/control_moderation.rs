use std::sync::Arc;

use super::ingest_outcome::{Ingested, ShellFollowUp};
use crate::state::AppState;

/// The media, preview and moderation arms of a relayed Control (plan GI4):
/// voice signaling, channel video and link previews become follow-ups for
/// the adapters that keep an `AppHandle`; a kick goes to the Host here.
pub(crate) fn moderation_core(
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    payload: rekindle_codec::community::envelope::ControlPayload,
) -> Ingested {
    use rekindle_codec::community::envelope::ControlPayload;

    let (community_id, sender) = (community_id.to_owned(), sender_pseudonym.to_owned());
    match payload {
        payload if rekindle_voice::signaling::is_voice_signaling(&payload) => {
            Ingested::follow_up(ShellFollowUp::VoiceSignaling {
                community_id,
                sender_pseudonym: sender,
                payload,
            })
        }
        payload @ (ControlPayload::VideoFragment { .. }
        | ControlPayload::VideoParityFragment { .. }
        | ControlPayload::KeyframeRequest { .. }
        | ControlPayload::TopologyChange { .. }
        | ControlPayload::MediaCapabilities { .. }) => {
            Ingested::follow_up(ShellFollowUp::VideoPayload {
                community_id,
                sender_pseudonym: sender,
                payload,
            })
        }
        ControlPayload::LinkPreview {
            channel_id,
            message_id,
            url,
            title,
            description,
            site_name,
            fetched_at,
        } => Ingested::follow_up(ShellFollowUp::LinkPreview {
            community_id,
            sender_pseudonym: sender,
            channel_id,
            preview: rekindle_types::link_preview::LinkPreview {
                message_id,
                url,
                title,
                description,
                site_name,
                fetched_at,
            },
        }),
        other => {
            handle_gossip_moderation(state, &community_id, &sender, other);
            Ingested::default()
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
