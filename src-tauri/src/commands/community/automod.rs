use tauri::State;

use crate::state::SharedState;
use rekindle_protocol::dht::community::permissions_v2::Permissions;

use super::helpers::{random_16_bytes, require_permission};

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
    Ok(crate::services::community::automod::list_rules(state.inner(), &community_id)?
        .into_iter()
        .map(|rule| AutoModRuleDto {
            rule_id: rule.rule_id,
            name: rule.name,
            enabled: rule.enabled,
            keywords: rule.keywords,
            regex_patterns: rule.regex_patterns,
            action: rule.action,
            lamport: rule.lamport,
        })
        .collect())
}

#[tauri::command]
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
    require_permission(state.inner(), &community_id, Permissions::MANAGE_COMMUNITY)?;
    let trigger_json = serde_json::to_string(&serde_json::json!({
        "keywords": keywords,
        "regexPatterns": regex_patterns,
    }))
    .map_err(|e| format!("serialize trigger: {e}"))?;
    let rule_id_bytes = rule_id
        .as_deref()
        .map(hex::decode)
        .transpose()
        .map_err(|e| format!("invalid rule id: {e}"))?
        .and_then(|bytes| bytes.try_into().ok())
        .unwrap_or_else(random_16_bytes);
    let lamport = crate::state_helpers::increment_lamport(state.inner(), &community_id);
    crate::services::community::write_entry(
        state.inner(),
        &community_id,
        rekindle_types::governance::GovernanceEntry::AutoModRule {
            rule_id: rule_id_bytes,
            name,
            enabled,
            trigger_json,
            action,
            lamport,
        },
    )
    .await?;
    Ok(hex::encode(rule_id_bytes))
}

#[tauri::command]
pub async fn delete_automod_rule(
    state: State<'_, SharedState>,
    community_id: String,
    rule_id: String,
) -> Result<(), String> {
    require_permission(state.inner(), &community_id, Permissions::MANAGE_COMMUNITY)?;
    let rule_id_bytes: [u8; 16] = hex::decode(&rule_id)
        .map_err(|e| format!("invalid rule id: {e}"))?
        .try_into()
        .map_err(|_| "invalid rule id".to_string())?;
    let existing =
        crate::services::community::automod::get_rule(state.inner(), &community_id, &rule_id_bytes)?;
    let trigger_json = serde_json::to_string(&serde_json::json!({
        "keywords": existing.keywords,
        "regexPatterns": existing.regex_patterns,
    }))
    .map_err(|e| format!("serialize trigger: {e}"))?;
    let lamport = crate::state_helpers::increment_lamport(state.inner(), &community_id);
    crate::services::community::write_entry(
        state.inner(),
        &community_id,
        rekindle_types::governance::GovernanceEntry::AutoModRule {
            rule_id: rule_id_bytes,
            name: existing.name,
            enabled: false,
            trigger_json,
            action: existing.action,
            lamport,
        },
    )
    .await
}
