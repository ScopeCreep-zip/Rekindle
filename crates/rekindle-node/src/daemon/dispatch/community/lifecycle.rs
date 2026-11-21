//! Community lifecycle: create, join, leave.

use crate::daemon::DaemonState;
use rekindle_ipc::protocol::IpcResponse;

use crate::daemon::dispatch::{adapter, state_error, DaemonContext};

pub(crate) async fn handle_create(
    ctx: &DaemonContext,
    state: DaemonState,
    name: &str,
    description: &str,
    approval_required: bool,
    co_creators: &[String],
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let name = name.trim().to_owned();

    // Refuse if a community with this name already exists in the session
    {
        let guard = ctx.session.read();
        if let Some(ref s) = *guard {
            if s.community_by_name(&name).is_some() {
                return IpcResponse::error(409, format!(
                    "community '{name}' already exists — use a different name or leave the existing one first",
                ));
            }
        }
    }

    // Genesis-only, so this is the one chance to set it: the merge
    // honours `AdmissionPolicy` solely as the genesis entry.
    let admission = if approval_required {
        rekindle_types::governance::AdmissionMode::ApprovalRequired
    } else {
        rekindle_types::governance::AdmissionMode::Open
    };
    // The creator-set round's stages are a large future: boxed off the
    // dispatch path.
    let created =
        match Box::pin(adapter(ctx).create(&name, description, admission, co_creators)).await {
            Ok(created) => created,
            Err(e) => return IpcResponse::error(500, e),
        };
    // The shell starts its own community services on `CommunityCreated`;
    // a creation waiting on co-creators has no community yet.
    if ctx.shell.is_none() && created.pending_co_creators.is_empty() {
        let membership = ctx
            .session
            .read()
            .as_ref()
            .and_then(|s| s.community(&created.anchor).cloned());
        if let Some(membership) = membership {
            activate_community(ctx, &membership).await;
        }
    }
    IpcResponse::ok(&created)
}

/// Bring a community this unlock created or joined up as unlock brings up
/// each resumed one (`dispatch::lifecycle`): its subscription mesh (the
/// message clock lives there) and its gossip mesh. Its record keepalive and
/// presence poll start from the records hand-over the create or join
/// already ran (`community_records_ready_impl`, finding IB1).
pub(crate) async fn activate_community(
    ctx: &DaemonContext,
    membership: &rekindle_transport::session::CommunityMembership,
) {
    let community_id = membership.anchor.clone();
    let subscriptions = ctx.subscriptions.read().clone();
    if let Some(subscriptions) = subscriptions {
        subscriptions.setup_community(membership);
    }
    if let Some(broadcast) = ctx.broadcast_mgr.read().as_ref() {
        broadcast.register_mesh(&community_id);
    }
    tracing::info!(community = %community_id, "community activated");
}

/// A creator-set round request of plan E3.2 S7s.3.
pub(crate) enum CreatorRound {
    Remove { anchor: String, identity: String },
    Cancel { anchor: String },
    Answer { anchor: String, accept: bool },
}

pub(crate) async fn handle_creator_round(
    ctx: &DaemonContext,
    state: DaemonState,
    round: CreatorRound,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let governance = adapter(ctx);
    let done = Box::pin(async {
        match round {
            CreatorRound::Remove { anchor, identity } => {
                governance.remove_co_creator(&anchor, &identity).await
            }
            CreatorRound::Cancel { anchor } => governance.cancel_creation(&anchor).await,
            CreatorRound::Answer { anchor, accept } => {
                governance.answer_creator_invite(&anchor, accept).await
            }
        }
    })
    .await;
    match done {
        Ok(()) => IpcResponse::ok(&serde_json::json!({})),
        Err(e) => IpcResponse::error(409, e),
    }
}

pub(crate) async fn handle_join(
    ctx: &DaemonContext,
    state: DaemonState,
    invite: &str,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    // The seed rides in the invite, so a full link is required (checked by
    // `validation::community` before the gate).
    let link = match rekindle_types::invite::InviteLink::parse(invite) {
        Ok(link) => link,
        Err(e) => return IpcResponse::error(400, format!("community invite: {e}")),
    };
    if let Err(e) = ctx.require_session(|_| ()) {
        return e;
    }
    // The join's stages are a large future: boxed off the dispatch path.
    let joined = match Box::pin(adapter(ctx).join(&link)).await {
        Ok(joined) => joined,
        Err(failure) => return join_failure_response(failure),
    };
    // The shell starts its own community services when the runtime hands
    // the records over (`ShellHooks::community_records_ready`).
    if ctx.shell.is_none() {
        let membership = ctx
            .session
            .read()
            .as_ref()
            .and_then(|s| s.community(&joined.anchor).cloned());
        if let Some(membership) = membership {
            activate_community(ctx, &membership).await;
        }
    }
    IpcResponse::ok(&joined)
}

pub(crate) async fn handle_leave(
    ctx: &DaemonContext,
    state: DaemonState,
    anchor: &str,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let membership = match ctx.resolve_community(anchor) {
        Ok(m) => m,
        Err(e) => return e,
    };
    match adapter(ctx).leave(&membership).await {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "left": membership.anchor })),
        Err(e) => IpcResponse::error(500, e),
    }
}

/// Forget a community we were removed from (plan E3.2 S9 R3-5); 409 for
/// one we still belong to.
pub(crate) async fn handle_forget(
    ctx: &DaemonContext,
    state: DaemonState,
    anchor: &str,
) -> IpcResponse {
    use crate::host::adapters::governance::ForgetError;
    if !state.can_write() {
        return state_error(state, "write");
    }
    let membership = match ctx.resolve_community(anchor) {
        Ok(m) => m,
        Err(e) => return e,
    };
    match adapter(ctx).forget(&membership.anchor).await {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "forgotten": membership.anchor })),
        Err(ForgetError::NotRemoved) => IpcResponse::error_with_remediation(
            409,
            "only a community you were removed from can be forgotten",
            "leave it instead (rekindle community leave)",
        ),
        Err(ForgetError::Failed(e)) => IpcResponse::error(500, e),
    }
}

pub(crate) async fn handle_update_info(
    ctx: &DaemonContext,
    state: DaemonState,
    anchor: &str,
    edit: crate::host::adapters::governance::InfoEdit,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    if let Err(e) = ctx.require_transport() {
        return e;
    }
    let membership = match ctx.resolve_community(anchor) {
        Ok(m) => m,
        Err(e) => return e,
    };
    match adapter(ctx).update_info(&membership.anchor, edit).await {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "updated": membership.anchor })),
        Err(e) => IpcResponse::error(500, e),
    }
}

/// The Host call's answer to a failed join. The inviter's typed refusal
/// (plan E3.2 join step 5, ADR 0019) keeps its code and what the user is
/// told, rendered as is; anything else is a 500.
pub(super) fn join_failure_response(
    failure: crate::host::adapters::governance::JoinFailure,
) -> IpcResponse {
    match failure {
        crate::host::adapters::governance::JoinFailure::Refused(reason) => IpcResponse::Error {
            code: reason.code(),
            message: reason.message().to_owned(),
            remediation: Some(reason.remediation().to_owned()),
        },
        crate::host::adapters::governance::JoinFailure::Failed(e) => IpcResponse::error(500, e),
    }
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;
