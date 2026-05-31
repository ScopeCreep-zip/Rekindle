use tauri::State;

use crate::db::DbPool;
use crate::services::community_onboarding_runtime::{
    get_onboarding_config_inner, get_welcome_screen_inner, mark_onboarding_complete_inner,
    set_onboarding_config_inner, set_welcome_screen_inner, submit_onboarding_answers_inner,
};
use crate::state::SharedState;

#[tauri::command]
pub async fn get_onboarding_config(
    state: State<'_, SharedState>,
    community_id: String,
) -> Result<serde_json::Value, String> {
    get_onboarding_config_inner(state.inner(), &community_id)
}

#[tauri::command]
pub async fn set_onboarding_config(
    state: State<'_, SharedState>,
    community_id: String,
    config: serde_json::Value,
) -> Result<(), String> {
    set_onboarding_config_inner(state.inner(), community_id, config).await
}

#[tauri::command]
pub async fn get_welcome_screen(
    state: State<'_, SharedState>,
    community_id: String,
) -> Result<serde_json::Value, String> {
    get_welcome_screen_inner(state.inner(), &community_id)
}

#[tauri::command]
pub async fn set_welcome_screen(
    state: State<'_, SharedState>,
    community_id: String,
    screen: serde_json::Value,
) -> Result<(), String> {
    set_welcome_screen_inner(state.inner(), community_id, screen).await
}

#[tauri::command]
pub async fn submit_onboarding_answers(
    state: State<'_, SharedState>,
    community_id: String,
    answers: Vec<serde_json::Value>,
    acknowledged_rules: Option<bool>,
) -> Result<(), String> {
    submit_onboarding_answers_inner(
        state.inner(),
        community_id,
        answers,
        acknowledged_rules.unwrap_or(false),
    )
    .await
}

#[tauri::command]
pub async fn mark_onboarding_complete(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
) -> Result<(), String> {
    mark_onboarding_complete_inner(state.inner(), pool.inner(), community_id).await
}
