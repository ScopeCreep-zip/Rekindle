//! The selection of a channel and category request (plans D3.3, E3.1).

use super::{
    channel_delete_escalations, channel_name, community, members, ChannelRequest, DaemonContext,
    IntentKind, IpcResponse, Reversibility, Selection, Subject,
};

/// `selection` for the channel and category requests.
pub(super) async fn selection(
    ctx: &DaemonContext,
    request: &ChannelRequest,
    s: &mut Selection,
) -> Result<(), IpcResponse> {
    match request {
        ChannelRequest::ChannelCreate {
            community: target,
            name,
            ..
        } => {
            let (id, community_name) = community(ctx, target)?;
            s.subject = Subject::Channel {
                community_name,
                id: String::new(),
                name: name.clone(),
            };
            s.reversibility = Reversibility::ReversibleByInverse(IntentKind::DeleteChannel);
            s.notifies = members(ctx, &id);
        }
        ChannelRequest::ChannelDelete {
            community: target,
            channel_id: channel,
        }
        | ChannelRequest::ChannelUpdate {
            community: target,
            channel_id: channel,
            ..
        }
        | ChannelRequest::ChannelRename {
            community: target,
            channel_id: channel,
            ..
        }
        | ChannelRequest::ChannelMove {
            community: target,
            channel_id: channel,
            ..
        }
        | ChannelRequest::ChannelSetTopic {
            community: target,
            channel_id: channel,
            ..
        }
        | ChannelRequest::ChannelSetSlowmode {
            community: target,
            channel_id: channel,
            ..
        }
        | ChannelRequest::ChannelSetForumTags {
            community: target,
            channel_id: channel,
            ..
        }
        | ChannelRequest::ChannelOverwriteSet {
            community: target,
            channel_id: channel,
            ..
        }
        | ChannelRequest::ChannelOverwriteDelete {
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
            if matches!(request, ChannelRequest::ChannelUpdate { .. }) {
                s.reversibility = Reversibility::Undoable;
            }

            if matches!(request, ChannelRequest::ChannelDelete { .. }) {
                s.escalations = channel_delete_escalations(ctx, &id, channel).await?;
            }
        }
        ChannelRequest::ChannelReorder {
            community: target, ..
        }
        | ChannelRequest::CategoryCreate {
            community: target, ..
        }
        | ChannelRequest::CategoryDelete {
            community: target, ..
        }
        | ChannelRequest::CategoryRename {
            community: target, ..
        }
        | ChannelRequest::CategoryReorder {
            community: target, ..
        } => {
            let (id, name) = community(ctx, target)?;
            if matches!(request, ChannelRequest::CategoryCreate { .. }) {
                s.reversibility = Reversibility::ReversibleByInverse(IntentKind::DeleteCategory);
            }
            s.notifies = members(ctx, &id);
            s.subject = Subject::Community { id, name };
        }
        _ => {}
    }
    Ok(())
}
