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
        payload @ ControlPayload::ThreadMessageReceived { .. } => {
            handle_thread_payload(app_handle, state, community_id, payload);
        }
        ControlPayload::GovernanceOp { op } => {
            // The Host's op store and rebuild (plan E3.2).
            if let Some(host) = state.host.current() {
                host.governance().on_governance_op(community_id, op).await;
            }
        }
        ControlPayload::HeadAdvanced { hint } => {
            // A ring watcher's hint (plan E3.2 S7): the desktop's gossip
            // ingest rate-limits the sender before it gets here
            // (`receiver_limits.rs`).
            if let Some(host) = state.host.current() {
                host.governance()
                    .on_head_advanced(community_id, sender_pseudonym, &hint, |_| true);
            }
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
                    role_ids: role_ids.iter().map(|id| id.to_hex()).collect(),
                },
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

/// A thread reply: stored by the Host's channel adapter (plan E3.1 E3f),
/// which reads its body from the thread record; the shell shows the event
/// it returns. A thread's opening and archive are signed governance ops
/// (plan E3.2 S8-E).
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
