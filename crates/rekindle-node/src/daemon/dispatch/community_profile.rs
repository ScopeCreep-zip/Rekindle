//! Our per-community profile, the community's pictures and its policy
//! (plan E3.1 D4), answered by the Host's governance adapter on both hosts.

use rekindle_governance_runtime::GovernanceRuntimeError;
use rekindle_ipc::protocol::IpcResponse;
use rekindle_types::image_kind::ImageKind;

pub(crate) use crate::host::adapters::governance::community_pictures::PictureSlot;
pub(crate) use crate::host::adapters::governance::community_profile::{PolicyEdit, ProfileEdit};

use super::{adapter, state_error, DaemonContext};
use crate::daemon::DaemonState;

/// Make `edit` our profile in `community`.
pub(crate) fn handle_update_profile(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    edit: ProfileEdit,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    match adapter(ctx).update_profile(&membership.governance_key, edit) {
        Ok(()) => IpcResponse::ok(&()),
        Err(e) => IpcResponse::error(404, format!("community profile: {e}")),
    }
}

/// Cache an uploaded avatar or banner; answered with its hash.
pub(crate) async fn handle_set_picture(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    image: Vec<u8>,
    kind: ImageKind,
    slot: PictureSlot,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    match adapter(ctx)
        .set_community_picture(&membership.governance_key, image, kind, slot)
        .await
    {
        Ok(hash) => IpcResponse::ok(&hash),
        Err(e) => IpcResponse::error(400, format!("community picture: {e}")),
    }
}

/// A cached picture as a data URL, or `None`.
pub(crate) async fn handle_picture(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    hash: &str,
) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    match adapter(ctx)
        .community_picture_data_url(&membership.governance_key, hash)
        .await
    {
        Ok(url) => IpcResponse::ok(&url),
        Err(e) => IpcResponse::error(500, format!("community picture: {e}")),
    }
}

/// The community's policy.
pub(crate) fn handle_policy(ctx: &DaemonContext, state: DaemonState, community: &str) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    match adapter(ctx).community_policy(&membership.governance_key) {
        Ok(policy) => IpcResponse::ok(&policy),
        Err(e) => refused(&e, "community policy"),
    }
}

/// Write a new policy.
pub(crate) async fn handle_set_policy(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    edit: PolicyEdit,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    match adapter(ctx)
        .set_community_policy(&membership.governance_key, edit)
        .await
    {
        Ok(()) => IpcResponse::ok(&()),
        Err(e) => refused(&e, "community policy"),
    }
}

/// A runtime error as the answer: 403 when we lack the permission.
fn refused(e: &GovernanceRuntimeError, what: &str) -> IpcResponse {
    match e {
        GovernanceRuntimeError::PermissionDenied => IpcResponse::error(403, format!("{what}: {e}")),
        _ => IpcResponse::error(500, format!("{what}: {e}")),
    }
}
