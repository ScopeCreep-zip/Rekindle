//! A community's automod rule requests (plan E3.1 F3, D26), answered by
//! the Host's governance adapter on both hosts.

use rekindle_ipc::protocol::{CommunityRequest, IpcResponse};

use crate::host::adapters::governance::automod_rules::AutoModRuleEdit;

use super::{adapter, runtime_refused, state_error, DaemonContext};
use crate::daemon::DaemonState;

/// An automod request.
pub(crate) async fn handle(
    ctx: &DaemonContext,
    state: DaemonState,
    request: CommunityRequest,
) -> IpcResponse {
    let read = matches!(request, CommunityRequest::AutomodRules { .. });
    if read && !state.can_query() {
        return state_error(state, "query");
    }
    if !read && !state.can_write() {
        return state_error(state, "write");
    }
    let community = match &request {
        CommunityRequest::AutomodRules { community }
        | CommunityRequest::AutomodRuleSet { community, .. }
        | CommunityRequest::AutomodRuleDelete { community, .. } => community.clone(),
        other => {
            return IpcResponse::error(500, format!("{} is not an automod request", other.name()))
        }
    };
    let key = match ctx.resolve_community(&community) {
        Ok(m) => m.governance_key,
        Err(e) => return e,
    };
    let host = adapter(ctx);
    match request {
        CommunityRequest::AutomodRuleSet {
            rule_id,
            name,
            enabled,
            keywords,
            regex_patterns,
            action,
            ..
        } => {
            let edit = AutoModRuleEdit {
                rule_id,
                name,
                enabled,
                keywords,
                regex_patterns,
                action,
            };
            match host.set_automod_rule(&key, edit).await {
                Ok(id) => IpcResponse::ok(&id),
                Err(e) => runtime_refused(&e, "automod rule set"),
            }
        }
        CommunityRequest::AutomodRuleDelete { rule_id, .. } => {
            match host.delete_automod_rule(&key, &rule_id).await {
                Ok(()) => IpcResponse::ok(&()),
                Err(e) => runtime_refused(&e, "automod rule delete"),
            }
        }
        _ => match host.automod_rules(&key) {
            Ok(rules) => IpcResponse::ok(&rules),
            Err(e) => runtime_refused(&e, "automod rules"),
        },
    }
}
