//! Channel message handlers: Send, the message changes and the history
//! reads.

use crate::daemon::DaemonState;
use rekindle_ipc::protocol::{CommunityRequest, IpcResponse};

use crate::host::adapters::governance::channel_messages::MessageChange;

use super::channel::resolve_channel;
use super::{state_error, DaemonContext};

pub(crate) async fn handle_send(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    channel: &str,
    body: &str,
    reply_to: Option<String>,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    let Some((_, channel_id)) = resolve_channel(ctx, &membership.governance_key, channel) else {
        return IpcResponse::error(404, format!("channel '{channel}' not found"));
    };
    post(ctx, &membership.governance_key, &channel_id, body, reply_to).await
}

/// Post `body` to `channel` (a channel id; plan E3.1 E2b).
pub(crate) async fn handle_post(
    ctx: &DaemonContext,
    state: DaemonState,
    channel: &str,
    body: &str,
    reply_to: Option<String>,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let Some(community_id) =
        crate::host::adapters::governance::channel_messages::community_of_channel(ctx, channel)
    else {
        return IpcResponse::error(404, format!("channel '{channel}' not found"));
    };
    post(ctx, &community_id, channel, body, reply_to).await
}

/// The Host's channel send, on both modes, as a reply to the message at
/// id `reply_to` when given: the post's state and id.
async fn post(
    ctx: &DaemonContext,
    community_id: &str,
    channel_id: &str,
    body: &str,
    reply_to: Option<String>,
) -> IpcResponse {
    let channel = crate::host::adapters::channel::HostChannel::new(ctx);
    match channel.send(community_id, channel_id, body, reply_to).await {
        Ok(sent) => {
            ctx.communities
                .mark_channel_synced(community_id, channel_id);
            IpcResponse::ok(&serde_json::json!({
                "status": sent.status,
                "messageId": sent.message_id,
                "timestamp": sent.timestamp_ms,
            }))
        }
        Err(e) => crate::host::adapters::channel::refusal(&e),
    }
}

/// Edit or delete our message `message_id` in `channel` (plan E3.1 E2).
pub(crate) async fn handle_change(
    ctx: &DaemonContext,
    state: DaemonState,
    channel: &str,
    message_id: &str,
    change: MessageChange,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let Some(community_id) =
        crate::host::adapters::governance::channel_messages::community_of_channel(ctx, channel)
    else {
        return IpcResponse::error(404, format!("channel '{channel}' not found"));
    };
    match super::adapter(ctx)
        .change_channel_message(&community_id, channel, message_id, change)
        .await
    {
        Ok(()) => IpcResponse::ok(&serde_json::Value::Null),
        Err(e) => IpcResponse::error(500, format!("channel message change failed: {e}")),
    }
}

/// Route a channel message send, edit or delete (the community router's
/// channel-message arm) to its handler.
pub(crate) async fn handle(
    ctx: &DaemonContext,
    state: DaemonState,
    request: CommunityRequest,
) -> IpcResponse {
    match request {
        CommunityRequest::ChannelMessageSend {
            channel,
            body,
            reply_to,
        } => handle_post(ctx, state, &channel, &body, reply_to).await,
        CommunityRequest::ChannelMessageEdit {
            channel,
            message_id,
            body,
        } => {
            let change = MessageChange::Edit { body };
            handle_change(ctx, state, &channel, &message_id, change).await
        }
        CommunityRequest::ChannelMessageDelete {
            channel,
            message_id,
        } => handle_change(ctx, state, &channel, &message_id, MessageChange::Delete).await,
        _ => IpcResponse::error(400, "not a channel message request"),
    }
}

/// Read a page of a channel's history (plan E3.1 E2c): the router's arm
/// for `ChannelMessages` and `ChannelMessagesBefore`.
pub(crate) async fn handle_read(
    ctx: &DaemonContext,
    state: DaemonState,
    request: CommunityRequest,
) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    let (community_id, channel, before, limit) = match request {
        CommunityRequest::ChannelMessages { channel, limit } => {
            let Some(community_id) =
                crate::host::adapters::governance::channel_messages::community_of_channel(
                    ctx, &channel,
                )
            else {
                return IpcResponse::error(404, format!("channel '{channel}' not found"));
            };
            (community_id, channel, None, limit)
        }
        CommunityRequest::ChannelMessagesBefore {
            community,
            channel,
            before_timestamp,
            limit,
        } => match ctx.resolve_community(&community) {
            Ok(membership) => (
                membership.governance_key,
                channel,
                Some(before_timestamp),
                limit,
            ),
            Err(e) => return e,
        },
        _ => return IpcResponse::error(400, "not a channel history request"),
    };
    let channel_adapter = crate::host::adapters::channel::HostChannel::new(ctx);
    match channel_adapter
        .history(&community_id, &channel, before, limit)
        .await
    {
        Ok(messages) => IpcResponse::ok(&messages),
        Err(e) => {
            let code = match e {
                rekindle_channel::error::ChannelError::PermissionDenied(_) => 403,
                rekindle_channel::error::ChannelError::ChannelNotFound(_)
                | rekindle_channel::error::ChannelError::CommunityNotFound(_) => 404,
                _ => 500,
            };
            IpcResponse::error(code, format!("channel history: {e}"))
        }
    }
}
