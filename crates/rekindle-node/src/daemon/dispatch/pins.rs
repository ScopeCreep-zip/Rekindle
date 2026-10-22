//! A channel's pins, the community's attachment pins and the local message
//! search (plan E3.1 E3b), answered by the Host's governance adapter on
//! both hosts.

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
        PinRequest::ChannelPins { .. } | PinRequest::MessageSearch { .. }
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
        | PinRequest::AttachmentPin { community, .. } => community.clone(),
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
        other => IpcResponse::error(500, format!("{} is not a pin request", other.name())),
    }
}
