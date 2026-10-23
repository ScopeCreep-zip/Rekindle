//! A channel's pins, the community's attachment pins and the local message
//! search (plan E3.1 E3b), answered by the Host's governance adapter on
//! both hosts; the attachments and voice messages (E3d) by its files
//! adapter (`attachments`).

use rekindle_ipc::protocol::IpcResponse;
use rekindle_ipc::protocol::PinRequest;

use super::{adapter, runtime_refused, state_error, DaemonContext};
use crate::daemon::DaemonState;

/// A pin or search request.
pub(crate) async fn handle(
    ctx: &DaemonContext,
    state: DaemonState,
    request: PinRequest,
) -> IpcResponse {
    let read = matches!(
        request,
        PinRequest::ChannelPins { .. }
            | PinRequest::MessageSearch { .. }
            | PinRequest::AttachmentInfo { .. }
            | PinRequest::VoiceMessageAudio { .. }
    );
    if read && !state.can_query() {
        return state_error(state, "query");
    }
    if !read && !state.can_write() {
        return state_error(state, "write");
    }
    let host = adapter(ctx);
    let community = match &request {
        PinRequest::MessageSearch { request } => {
            return match host.search_messages(request.clone()).await {
                Ok(result) => IpcResponse::ok(&result),
                Err(e) => runtime_refused(&e, "message search"),
            };
        }
        PinRequest::ChannelPins { community, .. }
        | PinRequest::MessagePin { community, .. }
        | PinRequest::MessageUnpin { community, .. }
        | PinRequest::AttachmentPin { community, .. }
        | PinRequest::AttachmentUpload { community, .. }
        | PinRequest::AttachmentDownload { community, .. }
        | PinRequest::AttachmentInfo { community, .. }
        | PinRequest::VoiceMessageSend { community, .. }
        | PinRequest::VoiceMessageAudio { community, .. } => community.clone(),
    };
    let key = match ctx.resolve_community(&community) {
        Ok(m) => m.governance_key,
        Err(e) => return e,
    };
    match request {
        PinRequest::ChannelPins { channel_id, .. } => {
            match host.channel_pins(&key, &channel_id).await {
                Ok(pins) => IpcResponse::ok(&pins),
                Err(e) => runtime_refused(&e, "channel pins"),
            }
        }
        PinRequest::MessagePin {
            channel_id,
            message_id,
            ..
        } => match host.pin_message(&key, &channel_id, &message_id).await {
            Ok(()) => IpcResponse::ok(&()),
            Err(e) => runtime_refused(&e, "pin message"),
        },
        PinRequest::MessageUnpin {
            channel_id,
            message_id,
            ..
        } => match host.unpin_message(&key, &channel_id, &message_id).await {
            Ok(()) => IpcResponse::ok(&()),
            Err(e) => runtime_refused(&e, "unpin message"),
        },
        PinRequest::AttachmentPin {
            attachment_id,
            pinned,
            ..
        } => match host.pin_attachment(&key, &attachment_id, pinned).await {
            Ok(()) => IpcResponse::ok(&()),
            Err(e) => runtime_refused(&e, "pin attachment"),
        },
        request @ (PinRequest::AttachmentUpload { .. }
        | PinRequest::AttachmentDownload { .. }
        | PinRequest::AttachmentInfo { .. }
        | PinRequest::VoiceMessageSend { .. }
        | PinRequest::VoiceMessageAudio { .. }) => {
            super::attachments::handle(ctx, &key, request).await
        }
        other @ PinRequest::MessageSearch { .. } => {
            IpcResponse::error(500, format!("{} is not a pin request", other.name()))
        }
    }
}
