//! The selection of a channel message and channel key request (plans D3.3, E3.1).

use super::{
    channel_name, community, members, ChannelMessageRequest, DaemonContext, IpcResponse, Selection,
    Subject,
};

/// `selection` for the channel message and channel key requests.
pub(super) fn selection(
    ctx: &DaemonContext,
    request: &ChannelMessageRequest,
    s: &mut Selection,
) -> Result<(), IpcResponse> {
    match request {
        ChannelMessageRequest::ChannelSend {
            community: target,
            channel,
            ..
        }
        | ChannelMessageRequest::MekRotate {
            community: target,
            channel,
        } => {
            let (id, community_name) = community(ctx, target)?;
            s.subject = Subject::Channel {
                name: channel_name(ctx, &id, channel),
                community_name,
                id: channel.clone(),
            };
            s.notifies = members(ctx, &id);
        }
        ChannelMessageRequest::ChannelMessageEdit { channel, .. }
        | ChannelMessageRequest::ChannelMessageDelete { channel, .. }
        | ChannelMessageRequest::ChannelMessageSend { channel, .. } => {
            let target = crate::host::adapters::governance::channel_messages::community_of_channel(
                ctx, channel,
            )
            .ok_or_else(|| IpcResponse::error(404, format!("channel '{channel}' not found")))?;
            let (id, community_name) = community(ctx, &target)?;
            s.subject = Subject::Channel {
                name: channel_name(ctx, &id, channel),
                community_name,
                id: channel.clone(),
            };
            s.notifies = members(ctx, &id);
        }
        ChannelMessageRequest::ChannelMessageForward { .. }
        | ChannelMessageRequest::ChannelMessageAdminDelete { .. }
        | ChannelMessageRequest::ChannelMessageBulkDelete { .. } => {
            super::super::community_messages::selection(ctx, request, s)?;
        }
        _ => {}
    }
    Ok(())
}
