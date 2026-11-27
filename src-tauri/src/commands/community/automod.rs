//! A community's automod rules (plan E3.1 F3), answered by the Host.

use rekindle_ipc::protocol::MemberRequest;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::dto::community::AutoModRuleView;
use rekindle_types::dto::{CommandError, Done};
use rekindle_types::governance::AutoModAction;
use tauri::State;

use crate::state::SharedState;

host_command! {
    state;
    pub async fn list_automod_rules(
        state: State<'_, SharedState>,
        community_id: String,
    ) -> Result<Done<Vec<AutoModRuleView>>, CommandError> {
        IpcRequest::Community(CommunityRequest::Members(MemberRequest::AutomodRules {
            community: community_id,
        }))
    }
}

host_command! {
    state;
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
        IpcRequest::Community(CommunityRequest::Members(MemberRequest::AutomodRuleSet {
            community: community_id,
            rule_id,
            name,
            enabled,
            keywords,
            regex_patterns,
            action,
        }))
    }
}

host_command! {
    state;
    pub async fn delete_automod_rule(
        state: State<'_, SharedState>,
        community_id: String,
        rule_id: String,
    ) -> Result<Done<()>, CommandError> {
        IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::AutomodRuleDelete {
                community: community_id,
                rule_id,
            },
        ))
    }
}
