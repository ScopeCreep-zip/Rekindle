//! The selection of a pin and search request (plans D3.3, E3.1).

use super::{
    channel_name, community, members, DaemonContext, IntentKind, IpcResponse, PinRequest,
    Reversibility, Selection, Subject,
};

/// `selection` for the pin and search requests.
pub(super) fn selection(
    ctx: &DaemonContext,
    request: &PinRequest,
    s: &mut Selection,
) -> Result<(), IpcResponse> {
    match request {
        PinRequest::MessagePin {
            community: target,
            channel_id: channel,
            ..
        }
        | PinRequest::MessageUnpin {
            community: target,
            channel_id: channel,
            ..
        } => {
            let (id, community_name) = community(ctx, target)?;
            s.subject = Subject::Channel {
                name: channel_name(ctx, &id, channel),
                community_name,
                id: channel.clone(),
            };
            s.notifies = members(ctx, &id);

            if matches!(request, PinRequest::MessagePin { .. }) {
                s.reversibility = Reversibility::ReversibleByInverse(IntentKind::UnpinMessage);
            }
            if matches!(request, PinRequest::MessageUnpin { .. }) {
                s.reversibility = Reversibility::ReversibleByInverse(IntentKind::PinMessage);
            }
        }
        PinRequest::AttachmentPin {
            community: target, ..
        } => {
            let (id, name) = community(ctx, target)?;

            s.subject = Subject::Community { id, name };
        }
        _ => {}
    }
    Ok(())
}
