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
    let created = match adapter(ctx).create(&name, description, admission).await {
        Ok(created) => created,
        Err(e) => return IpcResponse::error(500, e),
    };
    // The shell starts its own community services on `CommunityCreated`.
    if ctx.shell.is_none() {
        let membership = ctx
            .session
            .read()
            .as_ref()
            .and_then(|s| s.community(&created.governance_key).cloned());
        if let Some(membership) = membership {
            activate_community(ctx, &membership).await;
        }
    }
    IpcResponse::ok(&created)
}

/// Bring a community this unlock created or joined up as unlock brings up
/// each resumed one (`dispatch::lifecycle`): its record keepalive, its
/// watches and subscription mesh (the message clock lives there), its
/// gossip mesh, and its presence poll.
pub(crate) async fn activate_community(
    ctx: &DaemonContext,
    membership: &rekindle_transport::session::CommunityMembership,
) {
    let community_id = membership.governance_key.clone();
    crate::daemon::keepalive::start(ctx, &community_id);
    let subscriptions = ctx.subscriptions.read().clone();
    if let Some(subscriptions) = subscriptions {
        subscriptions.setup_community(membership).await;
    }
    if let Some(broadcast) = ctx.broadcast_mgr.read().as_ref() {
        broadcast.register_mesh(&community_id);
    }
    let started = ctx.presence_start_tx.send(
        crate::daemon::presence_adapter::supervisor::PresenceStartRequest {
            community_ids: vec![community_id.clone()],
        },
    );
    if started.is_err() {
        tracing::warn!(community = %community_id, "presence supervisor is gone — no roster");
    }
    tracing::info!(community = %community_id, "community activated");
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
        Err(e) => return IpcResponse::error(500, e),
    };
    // The shell starts its own community services when the runtime hands
    // the records over (`ShellHooks::community_records_ready`).
    if ctx.shell.is_none() {
        let membership = ctx
            .session
            .read()
            .as_ref()
            .and_then(|s| s.community(&joined.governance_key).cloned());
        if let Some(membership) = membership {
            activate_community(ctx, &membership).await;
        }
    }
    IpcResponse::ok(&joined)
}

pub(crate) async fn handle_leave(
    ctx: &DaemonContext,
    state: DaemonState,
    governance_key: &str,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let membership = match ctx.resolve_community(governance_key) {
        Ok(m) => m,
        Err(e) => return e,
    };
    match adapter(ctx).leave(&membership).await {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "left": membership.governance_key })),
        Err(e) => IpcResponse::error(500, e),
    }
}

pub(crate) async fn handle_update_info(
    ctx: &DaemonContext,
    state: DaemonState,
    governance_key: &str,
    edit: crate::host::adapters::governance::InfoEdit,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    if let Err(e) = ctx.require_transport() {
        return e;
    }
    let membership = match ctx.resolve_community(governance_key) {
        Ok(m) => m,
        Err(e) => return e,
    };
    match adapter(ctx)
        .update_info(&membership.governance_key, edit)
        .await
    {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "updated": membership.governance_key })),
        Err(e) => IpcResponse::error(500, e),
    }
}
