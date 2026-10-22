use std::sync::Arc;

use crate::state::AppState;

/// An event control message, stored by the Host's governance adapter (plan
/// E3.1 E4) and shown when it accepted it.
pub(super) async fn handle_event_payload(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    payload: rekindle_codec::community::envelope::ControlPayload,
) {
    use rekindle_codec::community::envelope::ControlPayload;
    use rekindle_types::subscription_events::{SocialEvent, SubscriptionEvent};

    let Some(host) = state.host.current() else {
        tracing::debug!(community = %community_id, "event change before the Host is up — dropped");
        return;
    };
    if !host
        .governance()
        .on_event_control(community_id, sender_pseudonym, &payload)
        .await
    {
        return;
    }
    let community = community_id.to_string();
    let event = match payload {
        ControlPayload::EventCreated { event } => SocialEvent::EventCreated {
            community,
            event: Box::new(event),
        },
        ControlPayload::EventUpdated { event } => SocialEvent::EventUpdated {
            community,
            event: Box::new(event),
        },
        ControlPayload::EventDeleted { event_id } => SocialEvent::EventDeleted {
            community,
            event_id,
        },
        ControlPayload::EventRsvpChanged {
            event_id,
            pseudonym_key,
            status,
        } => SocialEvent::EventRsvpChanged {
            community,
            event_id,
            pseudonym: pseudonym_key,
            rsvp_status: status,
        },
        _ => return,
    };
    crate::event_dispatch::emit_subscription(app_handle, &SubscriptionEvent::Social(event));
}

/// A game-server control message, stored by the Host's governance adapter
/// (plan E3.1 H1) and shown when it accepted it.
pub(super) async fn handle_game_server_payload(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    payload: rekindle_codec::community::envelope::ControlPayload,
) {
    use rekindle_codec::community::envelope::ControlPayload;
    use rekindle_types::subscription_events::{SocialEvent, SubscriptionEvent};

    let Some(host) = state.host.current() else {
        tracing::debug!(community = %community_id, "game server change before the Host is up — dropped");
        return;
    };
    if !host
        .governance()
        .on_game_server_control(community_id, sender_pseudonym, &payload)
        .await
    {
        return;
    }
    let community = community_id.to_string();
    let event = match payload {
        ControlPayload::GameServerAdded { server } => SocialEvent::GameServerAdded {
            community,
            server: Box::new(server),
        },
        ControlPayload::GameServerRemoved { server_id } => SocialEvent::GameServerRemoved {
            community,
            server_id,
        },
        _ => return,
    };
    crate::event_dispatch::emit_subscription(app_handle, &SubscriptionEvent::Social(event));
}
