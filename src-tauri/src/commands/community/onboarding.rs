//! The onboarding and welcome-screen commands (plan E3.1 D3): host calls,
//! answered by the Host's governance adapter
//! (`rekindle-node`'s `host/adapters/governance/onboarding.rs`).

use rekindle_ipc::protocol::SettingsRequest;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::dto::onboarding::{OnboardingConfigDto, WelcomeScreenDto};
use rekindle_types::dto::{CommandError, Done};
use rekindle_types::presence::OnboardingAnswer;
use tauri::State;

use crate::host_call;
use crate::state::SharedState;

#[tauri::command]
pub async fn get_onboarding_config(
    state: State<'_, SharedState>,
    community_id: String,
) -> Result<Done<OnboardingConfigDto>, CommandError> {
    let request = CommunityRequest::Settings(SettingsRequest::OnboardingConfigGet {
        community: community_id,
    });
    host_call::call(&state, IpcRequest::Community(request)).await
}

#[tauri::command]
pub async fn set_onboarding_config(
    state: State<'_, SharedState>,
    community_id: String,
    config: OnboardingConfigDto,
) -> Result<Done<()>, CommandError> {
    let request = CommunityRequest::Settings(SettingsRequest::OnboardingConfigSet {
        community: community_id,
        config,
    });
    host_call::call(&state, IpcRequest::Community(request)).await
}

#[tauri::command]
pub async fn get_welcome_screen(
    state: State<'_, SharedState>,
    community_id: String,
) -> Result<Done<WelcomeScreenDto>, CommandError> {
    let request = CommunityRequest::Settings(SettingsRequest::WelcomeScreenGet {
        community: community_id,
    });
    host_call::call(&state, IpcRequest::Community(request)).await
}

#[tauri::command]
pub async fn set_welcome_screen(
    state: State<'_, SharedState>,
    community_id: String,
    screen: WelcomeScreenDto,
) -> Result<Done<()>, CommandError> {
    let request = CommunityRequest::Settings(SettingsRequest::WelcomeScreenSet {
        community: community_id,
        screen,
    });
    host_call::call(&state, IpcRequest::Community(request)).await
}

#[tauri::command]
pub async fn submit_onboarding_answers(
    state: State<'_, SharedState>,
    community_id: String,
    answers: Vec<OnboardingAnswer>,
    acknowledged_rules: bool,
) -> Result<Done<()>, CommandError> {
    let request = CommunityRequest::Settings(SettingsRequest::OnboardingSubmit {
        community: community_id,
        answers,
        acknowledged_rules,
    });
    host_call::call(&state, IpcRequest::Community(request)).await
}

#[tauri::command]
pub async fn mark_onboarding_complete(
    state: State<'_, SharedState>,
    community_id: String,
) -> Result<Done<()>, CommandError> {
    let request = CommunityRequest::Settings(SettingsRequest::OnboardingComplete {
        community: community_id,
    });
    host_call::call(&state, IpcRequest::Community(request)).await
}
