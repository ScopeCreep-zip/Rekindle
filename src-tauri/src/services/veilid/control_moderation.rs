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

    // Our own removal is the Host's, from the merged governance (plan E3.2
    // S9 own removal): unauthenticated gossip naming us changes nothing.
    if my_pseudonym.as_deref() == Some(&target_pseudonym) {
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
        // The gossip dispatcher stays sync; the Host's departure awaits its
        // own key-transfer discard (review S7m part 1 F3).
        Some(host) => {
            let (community, member) = (community_id.to_owned(), target_pseudonym.clone());
            tauri::async_runtime::spawn(async move {
                host.governance().forget_member(&community, &member).await;
            });
        }
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

/// Whether `sender_pseudonym` holds the permission a gossip moderation
/// payload needs (a `Kick` needs `KICK_MEMBERS`); other payloads pass.
fn check_gossip_moderation_permission(
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    payload: &rekindle_codec::community::envelope::ControlPayload,
) -> bool {
    use rekindle_codec::community::envelope::ControlPayload;
    use rekindle_types::permissions;

    let required: u64 = match payload {
        ControlPayload::Kick { .. } => permissions::KICK_MEMBERS,
        _ => return true,
    };

    let Some(perms) =
        crate::state_helpers::permissions_for(state, community_id, sender_pseudonym, None)
    else {
        return false;
    };
    if rekindle_governance::permissions::has_capability(perms, required) {
        true
    } else {
        tracing::warn!(
            community = %community_id,
            sender = %sender_pseudonym,
            required = format!("{required:#x}"),
            "gossip moderation: sender lacks required permission — ignoring"
        );
        false
    }
}
