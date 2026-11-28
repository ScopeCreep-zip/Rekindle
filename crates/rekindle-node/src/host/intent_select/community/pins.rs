//! The selection of a pin and search request (plans D3.3, E3.1).

use super::{
    channel_name, community, members, DaemonContext, Escalation, IntentKind, IpcResponse,
    PinRequest, Reversibility, Selection, Subject,
};
use rekindle_types::intent::AttachmentFacts;

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
        PinRequest::AttachmentUpload {
            community: target,
            channel_id: channel,
            path,
        } => {
            let path = std::path::Path::new(path);
            crate::host::adapters::files::ops::upload_source(path)
                .map_err(|e| IpcResponse::error(400, format!("upload attachment: {e}")))?;
            let size_bytes = std::fs::symlink_metadata(path)
                .map_err(|e| IpcResponse::error(400, format!("upload attachment: {e}")))?
                .len();
            channel_subject(ctx, target, channel, s)?;
            s.escalations.push(Escalation::Attachment(AttachmentFacts {
                file_name: path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                size_bytes,
                metadata_removed: false,
            }));
        }
        PinRequest::AttachmentDownload {
            community: target,
            channel_id: channel,
            ..
        }
        | PinRequest::VoiceMessageSend {
            community: target,
            channel_id: channel,
            ..
        } => channel_subject(ctx, target, channel, s)?,
        _ => {}
    }
    Ok(())
}

/// The channel `channel` of `target` as the subject, reaching its members.
fn channel_subject(
    ctx: &DaemonContext,
    target: &str,
    channel: &str,
    s: &mut Selection,
) -> Result<(), IpcResponse> {
    let (id, community_name) = community(ctx, target)?;
    s.subject = Subject::Channel {
        name: channel_name(ctx, &id, channel),
        community_name,
        id: channel.to_owned(),
    };
    s.notifies = members(ctx, &id);
    Ok(())
}
