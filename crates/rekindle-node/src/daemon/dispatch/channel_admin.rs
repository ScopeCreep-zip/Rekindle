//! Channel and category administration dispatch (plan E3.1 E1): rename,
//! move, reorder, topic, forum tags and permission overwrites of channels,
//! and the categories' create, delete, rename and reorder. The logic is
//! `HostGovernance::channel_admin`; this resolves the request's community
//! and ids.

use rekindle_governance_runtime::GovernanceRuntimeError;
use rekindle_ipc::protocol::ChannelRequest;
use rekindle_ipc::protocol::IpcResponse;
use rekindle_types::id::{CategoryId, ChannelId};

use super::channel::resolve_channel;
use super::{state_error, DaemonContext};
use crate::daemon::DaemonState;
use crate::host::adapters::governance::channel_admin::ChannelAdminOp;

fn id16(hex_id: &str) -> Option<[u8; 16]> {
    hex::decode(hex_id)
        .ok()
        .and_then(|bytes| <[u8; 16]>::try_from(bytes).ok())
}

/// A channel's hex id.
pub(super) fn channel_id(hex_id: &str) -> Option<ChannelId> {
    id16(hex_id).map(ChannelId)
}

fn category(hex_id: &str) -> Result<CategoryId, IpcResponse> {
    id16(hex_id)
        .map(CategoryId)
        .ok_or_else(|| IpcResponse::error(400, format!("category '{hex_id}' is not an id")))
}

/// The request's community.
fn community_of(request: &ChannelRequest) -> &str {
    match request {
        ChannelRequest::ChannelRename { community, .. }
        | ChannelRequest::ChannelMove { community, .. }
        | ChannelRequest::ChannelReorder { community, .. }
        | ChannelRequest::ChannelSetTopic { community, .. }
        | ChannelRequest::ChannelSetSlowmode { community, .. }
        | ChannelRequest::ChannelSetForumTags { community, .. }
        | ChannelRequest::ChannelOverwriteSet { community, .. }
        | ChannelRequest::ChannelOverwriteDelete { community, .. }
        | ChannelRequest::CategoryCreate { community, .. }
        | ChannelRequest::CategoryDelete { community, .. }
        | ChannelRequest::CategoryRename { community, .. }
        | ChannelRequest::CategoryReorder { community, .. } => community,
        other => unreachable!("{} is not a channel admin request", other.name()),
    }
}

/// The request as an operation on `community_id`, its ids resolved.
fn op(
    ctx: &DaemonContext,
    community_id: &str,
    request: ChannelRequest,
) -> Result<ChannelAdminOp, IpcResponse> {
    let channel = |needle: &str| {
        resolve_channel(ctx, community_id, needle)
            .map(|(id, _)| id)
            .ok_or_else(|| IpcResponse::error(404, format!("channel '{needle}' not found")))
    };
    Ok(match request {
        ChannelRequest::ChannelRename {
            channel_id, name, ..
        } => ChannelAdminOp::Rename {
            channel: channel(&channel_id)?,
            name: name.trim().to_owned(),
        },
        ChannelRequest::ChannelMove {
            channel_id,
            category: target,
            ..
        } => ChannelAdminOp::Move {
            channel: channel(&channel_id)?,
            category: target.as_deref().map(category).transpose()?,
        },
        ChannelRequest::ChannelReorder { channel_ids, .. } => ChannelAdminOp::Reorder {
            channels: channel_ids
                .iter()
                .map(|id| channel(id))
                .collect::<Result<_, _>>()?,
        },
        ChannelRequest::ChannelSetSlowmode {
            channel_id,
            seconds,
            ..
        } => ChannelAdminOp::SetSlowmode {
            channel: channel(&channel_id)?,
            seconds,
        },
        ChannelRequest::ChannelSetTopic {
            channel_id, topic, ..
        } => ChannelAdminOp::SetTopic {
            channel: channel(&channel_id)?,
            topic,
        },
        ChannelRequest::ChannelSetForumTags {
            channel_id,
            forum_tags,
            ..
        } => ChannelAdminOp::SetForumTags {
            channel: channel(&channel_id)?,
            tags: forum_tags.iter().map(|t| t.trim().to_owned()).collect(),
        },
        ChannelRequest::ChannelOverwriteSet {
            channel_id,
            target_type,
            target_id,
            allow,
            deny,
            ..
        } => ChannelAdminOp::OverwriteSet {
            channel: channel(&channel_id)?,
            target_type,
            target_id,
            allow,
            deny,
        },
        ChannelRequest::ChannelOverwriteDelete {
            channel_id,
            target_type,
            target_id,
            ..
        } => ChannelAdminOp::OverwriteDelete {
            channel: channel(&channel_id)?,
            target_type,
            target_id,
        },
        ChannelRequest::CategoryCreate { name, .. } => ChannelAdminOp::CategoryCreate {
            name: name.trim().to_owned(),
        },
        ChannelRequest::CategoryDelete { category_id, .. } => ChannelAdminOp::CategoryDelete {
            category: category(&category_id)?,
        },
        ChannelRequest::CategoryRename {
            category_id, name, ..
        } => ChannelAdminOp::CategoryRename {
            category: category(&category_id)?,
            name: name.trim().to_owned(),
        },
        ChannelRequest::CategoryReorder { category_ids, .. } => ChannelAdminOp::CategoryReorder {
            categories: category_ids
                .iter()
                .map(|id| category(id))
                .collect::<Result<_, _>>()?,
        },
        other => unreachable!("{} is not a channel admin request", other.name()),
    })
}

/// Route one channel or category administration request.
pub(super) async fn route(
    ctx: &DaemonContext,
    state: DaemonState,
    request: ChannelRequest,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let membership = match ctx.resolve_community(community_of(&request)) {
        Ok(m) => m,
        Err(e) => return e,
    };
    let op = match op(ctx, &membership.governance_key, request) {
        Ok(op) => op,
        Err(e) => return e,
    };
    match super::adapter(ctx)
        .channel_admin(&membership.governance_key, op)
        .await
    {
        Ok(answer) => IpcResponse::ok(&answer),
        Err(GovernanceRuntimeError::PermissionDenied) => {
            IpcResponse::error(403, "not permitted in this community")
        }
        Err(e) => IpcResponse::error(500, format!("channel admin failed: {e}")),
    }
}
