use std::sync::Arc;

use crate::state::AppState;
use crate::state_helpers;

use super::control_event_records::{
    handle_event_payload, handle_game_server_payload, handle_pin_payload,
};
use super::control_moderation::moderation_core;
use super::ingest_outcome::{IngestOutcome, ShellFollowUp};

/// The record, thread and governance arms of a relayed Control (plan GI4):
/// the Host's accepted pin, event and game-server changes become events, a
/// thread reply a follow-up; the rest goes to `moderation_core`.
pub(crate) async fn control_events_core(
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    payload: rekindle_codec::community::envelope::ControlPayload,
) -> IngestOutcome {
    use rekindle_codec::community::envelope::ControlPayload;

    match payload {
        payload @ (ControlPayload::MessagePinned { .. }
        | ControlPayload::MessageUnpinned { .. }) => IngestOutcome::social(
            handle_pin_payload(state, community_id, sender_pseudonym, payload).await,
        ),
        payload @ (ControlPayload::EventCreated { .. }
        | ControlPayload::EventUpdated { .. }
        | ControlPayload::EventDeleted { .. }
        | ControlPayload::EventRsvpChanged { .. }) => IngestOutcome::social(
            handle_event_payload(state, community_id, sender_pseudonym, payload).await,
        ),
        payload @ ControlPayload::ThreadMessageReceived { .. } => {
            IngestOutcome::follow_up(ShellFollowUp::ThreadControl {
                community_id: community_id.to_owned(),
                sender_pseudonym: sender_pseudonym.to_owned(),
                payload,
            })
        }
        ControlPayload::GovernanceOp { op } => {
            // The Host's op store and rebuild (plan E3.2).
            if let Some(host) = state.host.current() {
                host.governance().on_governance_op(community_id, op).await;
            }
            IngestOutcome::default()
        }
        payload @ (ControlPayload::GameServerAdded { .. }
        | ControlPayload::GameServerRemoved { .. }) => IngestOutcome::social(
            handle_game_server_payload(state, community_id, sender_pseudonym, payload).await,
        ),
        payload @ ControlPayload::SubmitOnboardingAnswers { .. } => {
            super::control::host_membership(state, community_id, sender_pseudonym, &payload).await;
            IngestOutcome::default()
        }
        // Unauthenticated notices the shell shows nothing from: a window's
        // `MekRotated` comes from the Host installing the key
        // (`mek_transfer.rs`) or from our own rotation (`mek_adapter.rs`),
        // and `OnboardingCompleted` from the Host's own validation of the
        // answers (`membership_events/onboarding.rs`), with the roles it
        // assigned merging as `RolesChanged`.
        ControlPayload::MEKRotated { .. } | ControlPayload::OnboardingComplete { .. } => {
            IngestOutcome::default()
        }
        other => moderation_core(state, community_id, sender_pseudonym, other),
    }
}

/// A thread reply from its authenticated `sender_pseudonym`: stored by the
/// Host's channel adapter (plan E3.1 E3f) only from that sender's verified
/// entry in the thread record; the shell shows the event it returns. A thread's opening and archive are signed governance ops
/// (plan E3.2 S8-E).
pub(super) fn handle_thread_payload(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    payload: rekindle_codec::community::envelope::ControlPayload,
) {
    let Some(host) = state.host.current() else {
        tracing::debug!(community = %community_id, "thread change before the Host is up — dropped");
        return;
    };
    let scope = state_helpers::login_scope_or_closed(state);
    let (app_handle, community_id, sender, task_scope) = (
        app_handle.clone(),
        community_id.to_string(),
        sender_pseudonym.to_string(),
        Arc::clone(&scope),
    );
    scope.spawn_or_drop("thread control", async move {
        if let Some(event) = host
            .channel()
            .on_thread_control(&community_id, &sender, &payload, &task_scope)
            .await
        {
            crate::event_dispatch::emit_subscription(
                &app_handle,
                &rekindle_types::subscription_events::SubscriptionEvent::Social(event),
            );
        }
    });
}
