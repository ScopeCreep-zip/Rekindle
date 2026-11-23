use std::sync::Arc;

use rekindle_codec::community::envelope::ControlPayload;
use rekindle_types::subscription_events::SocialEvent;

use crate::state::AppState;

/// An event control message, stored by the Host's governance adapter (plan
/// E3.1 E4); the event to show when it accepted it (plan GI4).
pub(super) async fn handle_event_payload(
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    payload: ControlPayload,
) -> Option<SocialEvent> {
    let Some(host) = state.host.current() else {
        tracing::debug!(community = %community_id, "event change before the Host is up — dropped");
        return None;
    };
    if !host
        .governance()
        .on_event_control(community_id, sender_pseudonym, &payload)
        .await
    {
        return None;
    }
    let community = community_id.to_string();
    match payload {
        ControlPayload::EventCreated { event } => Some(SocialEvent::EventCreated {
            community,
            event: Box::new(event),
        }),
        ControlPayload::EventUpdated { event } => Some(SocialEvent::EventUpdated {
            community,
            event: Box::new(event),
        }),
        ControlPayload::EventDeleted { event_id } => Some(SocialEvent::EventDeleted {
            community,
            event_id,
        }),
        ControlPayload::EventRsvpChanged {
            event_id,
            pseudonym_key,
            status,
        } => Some(SocialEvent::EventRsvpChanged {
            community,
            event_id,
            pseudonym: pseudonym_key,
            rsvp_status: status,
        }),
        _ => None,
    }
}

/// A game-server control message, stored by the Host's governance adapter
/// (plan E3.1 H1); the event to show when it accepted it (plan GI4).
pub(super) async fn handle_game_server_payload(
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    payload: ControlPayload,
) -> Option<SocialEvent> {
    let Some(host) = state.host.current() else {
        tracing::debug!(community = %community_id, "game server change before the Host is up — dropped");
        return None;
    };
    if !host
        .governance()
        .on_game_server_control(community_id, sender_pseudonym, &payload)
        .await
    {
        return None;
    }
    let community = community_id.to_string();
    match payload {
        ControlPayload::GameServerAdded { server } => Some(SocialEvent::GameServerAdded {
            community,
            server: Box::new(server),
        }),
        ControlPayload::GameServerRemoved { server_id } => Some(SocialEvent::GameServerRemoved {
            community,
            server_id,
        }),
        _ => None,
    }
}

/// A message pin or unpin, stored by the Host's governance adapter (plan
/// E3.1 E3b); the event to show when it accepted it (plan GI4).
pub(super) async fn handle_pin_payload(
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    payload: ControlPayload,
) -> Option<SocialEvent> {
    let Some(host) = state.host.current() else {
        tracing::debug!(community = %community_id, "pin change before the Host is up — dropped");
        return None;
    };
    if !host
        .governance()
        .on_pin_control(community_id, sender_pseudonym, &payload)
        .await
    {
        return None;
    }
    let community = community_id.to_string();
    match payload {
        ControlPayload::MessagePinned {
            channel_id,
            message_id,
            pinned_by,
        } => Some(SocialEvent::MessagePinned {
            community,
            channel: channel_id,
            message_id,
            pinned_by,
        }),
        ControlPayload::MessageUnpinned {
            channel_id,
            message_id,
        } => Some(SocialEvent::MessageUnpinned {
            community,
            channel: channel_id,
            message_id,
        }),
        _ => None,
    }
}
