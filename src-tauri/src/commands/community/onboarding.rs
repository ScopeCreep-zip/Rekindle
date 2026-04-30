use tauri::State;

use crate::state::SharedState;
use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_protocol::dht::community::permissions_v2::Permissions;

use super::helpers::{hex_to_id_16, require_permission};
use super::legacy::{
    governance_onboarding_to_manifest_shape, governance_welcome_to_protocol,
    onboarding_mode_to_string, protocol_guide_step_to_governance, protocol_question_to_governance,
    protocol_welcome_channel_to_governance,
};

#[tauri::command]
pub async fn get_onboarding_config(
    state: State<'_, SharedState>,
    community_id: String,
) -> Result<serde_json::Value, String> {
    let communities = state.communities.read();
    let config = communities
        .get(&community_id)
        .and_then(|community| community.governance_state.as_ref())
        .and_then(|gov| gov.onboarding.as_ref())
        .map(governance_onboarding_to_manifest_shape)
        .unwrap_or_default();
    serde_json::to_value(&config).map_err(|e| format!("serialize: {e}"))
}

#[tauri::command]
pub async fn set_onboarding_config(
    state: State<'_, SharedState>,
    community_id: String,
    config: serde_json::Value,
) -> Result<(), String> {
    require_permission(&state, &community_id, Permissions::MANAGE_COMMUNITY)?;
    let config: rekindle_protocol::dht::community::onboarding::OnboardingConfig =
        serde_json::from_value(config).map_err(|e| format!("invalid config: {e}"))?;

    let lamport = crate::state_helpers::increment_lamport(state.inner(), &community_id);
    crate::services::community::write_entry(
        state.inner(),
        &community_id,
        rekindle_types::governance::GovernanceEntry::OnboardingConfig {
            enabled: config.enabled,
            mode: onboarding_mode_to_string(config.mode),
            default_channels: config
                .default_channels
                .iter()
                .map(|id| rekindle_types::id::ChannelId(hex_to_id_16(id)))
                .collect(),
            questions: config
                .questions
                .into_iter()
                .map(protocol_question_to_governance)
                .collect(),
            welcome_message: config.welcome_message,
            guide_steps: config
                .guide_steps
                .into_iter()
                .map(protocol_guide_step_to_governance)
                .collect(),
            lamport,
        },
    )
    .await
}

#[tauri::command]
pub async fn get_welcome_screen(
    state: State<'_, SharedState>,
    community_id: String,
) -> Result<serde_json::Value, String> {
    let communities = state.communities.read();
    let screen = communities
        .get(&community_id)
        .ok_or("community not found")?
        .governance_state
        .as_ref()
        .and_then(|gov| gov.welcome_screen.as_ref())
        .map(governance_welcome_to_protocol)
        .unwrap_or_default();
    serde_json::to_value(&screen).map_err(|e| format!("serialize: {e}"))
}

#[tauri::command]
pub async fn set_welcome_screen(
    state: State<'_, SharedState>,
    community_id: String,
    screen: serde_json::Value,
) -> Result<(), String> {
    require_permission(&state, &community_id, Permissions::MANAGE_COMMUNITY)?;
    let screen: rekindle_protocol::dht::community::onboarding::WelcomeScreen =
        serde_json::from_value(screen).map_err(|e| format!("invalid screen: {e}"))?;
    let lamport = crate::state_helpers::increment_lamport(state.inner(), &community_id);
    crate::services::community::write_entry(
        state.inner(),
        &community_id,
        rekindle_types::governance::GovernanceEntry::WelcomeScreen {
            description: screen.description,
            channels: screen
                .channels
                .into_iter()
                .map(protocol_welcome_channel_to_governance)
                .collect(),
            lamport,
        },
    )
    .await
}

#[tauri::command]
pub async fn submit_onboarding_answers(
    state: State<'_, SharedState>,
    community_id: String,
    answers: Vec<serde_json::Value>,
) -> Result<(), String> {
    let answers: Vec<rekindle_protocol::dht::community::envelope::OnboardingAnswer> = answers
        .into_iter()
        .map(|v| serde_json::from_value(v).map_err(|e| format!("invalid answer: {e}")))
        .collect::<Result<Vec<_>, _>>()?;

    let envelope = CommunityEnvelope::Control(ControlPayload::SubmitOnboardingAnswers { answers });
    crate::services::community::send_to_mesh(state.inner(), &community_id, &envelope)
}
