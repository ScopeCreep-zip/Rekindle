//! A community's audit log and notification default (plan E3.1 F4),
//! answered by the Host's governance adapter on both hosts.

use rekindle_ipc::protocol::AuditRequest;
use rekindle_ipc::protocol::IpcResponse;

use super::{adapter, runtime_refused, state_error, DaemonContext};
use crate::daemon::DaemonState;

/// An audit-log or notification-default request.
pub(crate) async fn handle(
    ctx: &DaemonContext,
    state: DaemonState,
    request: AuditRequest,
) -> IpcResponse {
    match request {
        AuditRequest::AuditLog {
            community,
            before_timestamp,
            limit,
        } => handle_audit_log(ctx, state, &community, before_timestamp, limit).await,
        AuditRequest::NotificationDefaultGet { community } => {
            handle_notification_default_get(ctx, state, &community)
        }
        AuditRequest::NotificationDefaultSet { community, level } => {
            handle_notification_default_set(ctx, state, &community, &level).await
        }
    }
}

/// A page of the audit log.
async fn handle_audit_log(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    before_timestamp: Option<u64>,
    limit: u32,
) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    match adapter(ctx)
        .audit_log(&membership.governance_key, before_timestamp, limit)
        .await
    {
        Ok(rows) => IpcResponse::ok(&rows),
        Err(e) => runtime_refused(&e, "audit log"),
    }
}

/// The merged default notification level, or `null`.
fn handle_notification_default_get(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    IpcResponse::ok(&adapter(ctx).notification_default(&membership.governance_key))
}

/// Set the default notification level.
async fn handle_notification_default_set(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    level: &str,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    match adapter(ctx)
        .set_notification_default(&membership.governance_key, level)
        .await
    {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "level": level })),
        Err(e) => runtime_refused(&e, "notification default"),
    }
}
