//! A community's automod rules (plan E3.1 F3), answered by the Host.

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::dto::community::AutoModRuleView;
use rekindle_types::dto::{CommandError, Done};
use rekindle_types::governance::AutoModAction;
use tauri::State;

use crate::host_call;
use crate::state::SharedState;

#[tauri::command]
pub async fn list_automod_rules(
    state: State<'_, SharedState>,
    community_id: String,
) -> Result<Done<Vec<AutoModRuleView>>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::AutomodRules {
            community: community_id,
        }),
    )
    .await
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
    action: AutoModAction,
) -> Result<Done<String>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::AutomodRuleSet {
            community: community_id,
            rule_id,
            name,
            enabled,
            keywords,
            regex_patterns,
            action,
        }),
    )
    .await
}

#[tauri::command]
pub async fn delete_automod_rule(
    state: State<'_, SharedState>,
    community_id: String,
    rule_id: String,
) -> Result<Done<()>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::AutomodRuleDelete {
            community: community_id,
            rule_id,
        }),
    )
    .await
}
