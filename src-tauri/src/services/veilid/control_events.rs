use std::sync::Arc;

use crate::state::AppState;
use crate::state_helpers;

use super::control_event_records::{
    handle_event_payload, handle_game_server_payload, handle_pin_payload,
};
use super::control_moderation::handle_gossip_control_payloads;

pub(crate) async fn handle_control_events_and_threads(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    payload: rekindle_codec::community::envelope::ControlPayload,
) {
    use rekindle_codec::community::envelope::ControlPayload;

    match payload {
        ControlPayload::ChannelOverwriteChanged { channel_id } => {
            crate::event_dispatch::emit_subscription(
                app_handle,
                &rekindle_types::subscription_events::SubscriptionEvent::Governance(rekindle_types::subscription_events::GovernanceEvent::ChannelPermissionsChanged {
                    community: community_id.to_string(),
                    channel: channel_id,
                }),
            );
        }
        payload @ (ControlPayload::MessagePinned { .. }
        | ControlPayload::MessageUnpinned { .. }) => {
            handle_pin_payload(app_handle, state, community_id, sender_pseudonym, payload).await;
        }
        payload @ (ControlPayload::EventCreated { .. }
        | ControlPayload::EventUpdated { .. }
        | ControlPayload::EventDeleted { .. }
        | ControlPayload::EventRsvpChanged { .. }) => {
            handle_event_payload(app_handle, state, community_id, sender_pseudonym, payload).await;
        }
        payload @ (ControlPayload::ThreadCreated { .. }
        | ControlPayload::ThreadArchived { .. }
        | ControlPayload::ThreadMessageReceived { .. }) => {
            handle_thread_payload(app_handle, state, community_id, payload);
        }
        payload @ (ControlPayload::GameServerAdded { .. }
        | ControlPayload::GameServerRemoved { .. }) => {
            handle_game_server_payload(app_handle, state, community_id, sender_pseudonym, payload)
                .await;
        }
        ControlPayload::MEKRotated {
            channel_id,
            new_generation,
            ..
        } => {
            tracing::debug!(
                community = %community_id,
                "MEKRotated: v2.0 uses invite-time MEK distribution — vault read skipped"
            );
            crate::event_dispatch::emit_subscription(
                app_handle,
                &rekindle_types::subscription_events::SubscriptionEvent::Crypto(
                    rekindle_types::subscription_events::CryptoEvent::MekRotated {
                        community: community_id.to_string(),
                        channel: channel_id,
                        generation: new_generation,
                        // The gossip payload names no rotator.
                        rotator_pseudonym: None,
                    },
                ),
            );
        }
        ControlPayload::KickedNotification => {
            crate::event_dispatch::emit_subscription(
                app_handle,
                &rekindle_types::subscription_events::SubscriptionEvent::System(
                    rekindle_types::subscription_events::SystemEvent::Kicked {
                        community: community_id.to_string(),
                    },
                ),
            );
        }
        payload @ ControlPayload::SubmitOnboardingAnswers { .. } => {
            super::control::host_membership(state, community_id, sender_pseudonym, &payload).await;
        }
        ControlPayload::OnboardingComplete {
            ref pseudonym_key,
            ref role_ids,
        } => {
            crate::event_dispatch::emit_membership(
                app_handle,
                rekindle_types::subscription_events::MembershipEvent::OnboardingCompleted {
                    community: community_id.to_string(),
                    pseudonym: pseudonym_key.clone(),
                    role_ids: role_ids.clone(),
                },
            );
        }
        ControlPayload::EventReminder {
            event_id,
            title,
            minutes_until_start,
        } => {
            crate::event_dispatch::emit_subscription(
                app_handle,
                &rekindle_types::subscription_events::SubscriptionEvent::Social(
                    rekindle_types::subscription_events::SocialEvent::EventReminder {
                        community: community_id.to_string(),
                        event_id,
                        title,
                        minutes_until_start,
                    },
                ),
            );
        }
        ControlPayload::SystemMessage { body, timestamp } => {
            crate::event_dispatch::emit_subscription(
                app_handle,
                &rekindle_types::subscription_events::SubscriptionEvent::System(
                    rekindle_types::subscription_events::SystemEvent::Announcement {
                        // Tier 1's announcement is optionally global;
                        // a gossiped system message always has one.
                        community: Some(community_id.to_string()),
                        body,
                        timestamp,
                    },
                ),
            );
        }
        ControlPayload::RaidAlert { active } => {
            crate::event_dispatch::emit_subscription(
                app_handle,
                &rekindle_types::subscription_events::SubscriptionEvent::System(
                    rekindle_types::subscription_events::SystemEvent::RaidAlert {
                        community: community_id.to_string(),
                        active,
                    },
                ),
            );
        }
        ControlPayload::ChannelLockdown { locked } => {
            crate::event_dispatch::emit_subscription(
                app_handle,
                &rekindle_types::subscription_events::SubscriptionEvent::System(
                    rekindle_types::subscription_events::SystemEvent::ChannelLockdown {
                        community: community_id.to_string(),
                        locked,
                    },
                ),
            );
        }
        other => {
            handle_gossip_control_payloads(
                app_handle,
                state,
                community_id,
                sender_pseudonym,
                other,
            );
        }
    }
}

/// A thread opened, archived or replied to: stored by the Host's channel
/// adapter (plan E3.1 E3f), which reads a reply's body from the thread
/// record; the shell shows the event it returns.
fn handle_thread_payload(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    payload: rekindle_codec::community::envelope::ControlPayload,
) {
    let Some(host) = state.host.current() else {
        tracing::debug!(community = %community_id, "thread change before the Host is up — dropped");
        return;
    };
    let scope = state_helpers::login_scope_or_closed(state);
    let (app_handle, community_id, task_scope) = (
        app_handle.clone(),
        community_id.to_string(),
        Arc::clone(&scope),
    );
    scope.spawn_or_drop("thread control", async move {
        if let Some(event) = host
            .channel()
            .on_thread_control(&community_id, &payload, &task_scope)
            .await
        {
            crate::event_dispatch::emit_subscription(
                &app_handle,
                &rekindle_types::subscription_events::SubscriptionEvent::Social(event),
            );
        }
    });
}
