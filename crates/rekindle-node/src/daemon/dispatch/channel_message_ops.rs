//! Channel message forward, a moderator's deletes, typing and the active
//! channel (plan E3.1 E2d): each request resolved to its community and
//! channel, then run by the Host's channel or governance adapter.

use rekindle_ipc::protocol::ChannelMessageRequest;
use rekindle_ipc::protocol::IpcResponse;

use super::channel::resolve_channel;
use super::{state_error, DaemonContext};
use crate::daemon::DaemonState;

/// The community's governance key and the channel's hex id.
fn target(
    ctx: &DaemonContext,
    community: &str,
    channel: &str,
) -> Result<(String, String), IpcResponse> {
    let membership = ctx.resolve_community(community)?;
    let (_, channel_id) = resolve_channel(ctx, &membership.governance_key, channel)
        .ok_or_else(|| IpcResponse::error(404, format!("channel '{channel}' not found")))?;
    Ok((membership.governance_key, channel_id))
}

/// Route a forward, delete, typing or active-channel request.
pub(crate) async fn handle(
    ctx: &DaemonContext,
    state: DaemonState,
    request: ChannelMessageRequest,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    match run(ctx, request).await {
        Ok(response) | Err(response) => response,
    }
}

async fn run(
    ctx: &DaemonContext,
    request: ChannelMessageRequest,
) -> Result<IpcResponse, IpcResponse> {
    let governance = || super::adapter(ctx);
    Ok(match request {
        ChannelMessageRequest::ChannelMessageForward {
            source_community,
            source_channel,
            source_message,
            dest_community,
            dest_channel,
        } => {
            let (source_community, source_channel) =
                target(ctx, &source_community, &source_channel)?;
            let (dest_community, dest_channel) = target(ctx, &dest_community, &dest_channel)?;
            let channel = crate::host::adapters::channel::HostChannel::new(ctx);
            let sent = channel
                .forward(rekindle_channel::ForwardChannelMessageParams {
                    source_community: &source_community,
                    source_channel: &source_channel,
                    source_message: &source_message,
                    dest_community: &dest_community,
                    dest_channel: &dest_channel,
                })
                .await
                .map_err(|e| crate::host::adapters::channel::refusal(&e))?;
            ctx.communities
                .mark_channel_synced(&dest_community, &dest_channel);
            IpcResponse::ok(&serde_json::json!({
                "status": sent.status,
                "messageId": sent.message_id,
            }))
        }
        ChannelMessageRequest::ChannelMessageAdminDelete {
            community,
            channel,
            message_id,
            reason,
        } => {
            let (community, channel) = target(ctx, &community, &channel)?;
            governance()
                .admin_delete_message(&community, &channel, &message_id, reason.as_deref())
                .await
                .map_err(|e| IpcResponse::error(u32::from(e.code()), e.to_string()))?;
            IpcResponse::ok(&serde_json::Value::Null)
        }
        ChannelMessageRequest::ChannelMessageBulkDelete {
            community,
            channel,
            message_ids,
            reason,
        } => {
            let (community, channel) = target(ctx, &community, &channel)?;
            let deleted = governance()
                .bulk_delete_messages(&community, &channel, &message_ids, reason.as_deref())
                .await
                .map_err(|e| IpcResponse::error(u32::from(e.code()), e.to_string()))?;
            IpcResponse::ok(&deleted)
        }
        ChannelMessageRequest::ChannelTyping { community, channel } => {
            let (community, channel) = target(ctx, &community, &channel)?;
            governance()
                .send_typing(&community, &channel)
                .map_err(|e| IpcResponse::error(500, format!("typing: {e}")))?;
            IpcResponse::ok(&serde_json::Value::Null)
        }
        ChannelMessageRequest::ChannelSetActive {
            community,
            channel,
            kind,
        } => {
            let (community, channel) = match channel {
                Some(channel) => {
                    let (community, channel) = target(ctx, &community, &channel)?;
                    (community, Some(channel))
                }
                None => (ctx.resolve_community(&community)?.governance_key, None),
            };
            governance()
                .set_active_channel(&community, channel, kind == "voice")
                .map_err(|e| IpcResponse::error(404, format!("active channel: {e}")))?;
            IpcResponse::ok(&serde_json::Value::Null)
        }
        _ => IpcResponse::error(400, "not a channel message request"),
    })
}
