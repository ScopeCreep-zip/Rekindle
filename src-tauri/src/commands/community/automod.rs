use tauri::State;

use crate::services::community_automod_runtime::{
    delete_automod_rule_inner, list_automod_rules_inner, set_automod_rule_inner,
};
use crate::state::SharedState;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoModRuleDto {
    pub rule_id: String,
    pub name: String,
    pub enabled: bool,
    pub keywords: Vec<String>,
    pub regex_patterns: Vec<String>,
    pub action: String,
    pub lamport: u64,
}

#[tauri::command]
pub async fn list_automod_rules(
    state: State<'_, SharedState>,
    community_id: String,
) -> Result<Vec<AutoModRuleDto>, String> {
    list_automod_rules_inner(state.inner(), &community_id)
}

#[tauri::command]
#[allow(clippy::too_many_arguments, reason = "Tauri command surface — matches AutoModRule trigger payload")]
pub async fn set_automod_rule(
    state: State<'_, SharedState>,
    community_id: String,
    rule_id: Option<String>,
    name: String,
    enabled: bool,
    keywords: Vec<String>,
    regex_patterns: Vec<String>,
    action: String,
) -> Result<String, String> {
    set_automod_rule_inner(
        state.inner(),
        community_id,
        rule_id,
        name,
        enabled,
        keywords,
        regex_patterns,
        action,
    )
    .await
}

#[tauri::command]
pub async fn delete_automod_rule(
    state: State<'_, SharedState>,
    community_id: String,
    rule_id: String,
) -> Result<(), String> {
    delete_automod_rule_inner(state.inner(), community_id, rule_id).await
}
