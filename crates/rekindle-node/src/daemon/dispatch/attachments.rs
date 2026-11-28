//! A channel's attachments and voice messages (plan E3.1 E3d), answered by
//! the Host's files adapter on both hosts: an upload of the file the shell
//! picked, a download to the place it chose, an attachment's record, and a
//! voice message's send and audio. The paths are the shell's; an agent
//! never reaches here with one (`PinRequest::agent_capability`).

use std::path::Path;

use rekindle_files::FilesError;
use rekindle_ipc::protocol::{IpcResponse, PinRequest};

use super::{adapter, DaemonContext};
use crate::host::adapters::files::HostFiles;
use crate::host::transfers;
use rekindle_types::dto::transfer::TransferTarget;

/// The status a files failure answers with.
pub(super) fn refused(e: &FilesError, what: &str) -> IpcResponse {
    let code = match e {
        FilesError::FileTooLarge { .. }
        | FilesError::InvalidInput(_)
        | FilesError::InvalidAttachmentId(_)
        | FilesError::Slowmode { .. } => 400,
        FilesError::PermissionDenied(_) => 403,
        FilesError::NotFound(_) => 404,
        FilesError::IdentityNotLoaded | FilesError::MekUnavailable { .. } => 423,
        _ => 500,
    };
    IpcResponse::error(code, format!("{what}: {e}"))
}

/// An attachment or voice request, its community resolved to `key`.
pub(super) async fn handle(ctx: &DaemonContext, key: &str, request: PinRequest) -> IpcResponse {
    let files = HostFiles::new(adapter(ctx));
    match request {
        PinRequest::AttachmentUpload {
            channel_id, path, ..
        } => match files.upload(key, &channel_id, Path::new(&path)).await {
            Ok(id) => IpcResponse::ok(&id),
            Err(e) => refused(&e, "upload attachment"),
        },
        PinRequest::AttachmentDownload {
            channel_id,
            attachment_id,
            path,
            ..
        } => {
            // A transfer (plan E7.2 G12): Stop pauses it, Start resumes it.
            let target = TransferTarget::Channel {
                community: key.to_owned(),
                channel_id,
            };
            super::transfers::answer(
                transfers::download(ctx, target, &attachment_id, path.into()).await,
                "download attachment",
            )
        }
        PinRequest::AttachmentInfo {
            channel_id,
            attachment_id,
            ..
        } => match files.info(&channel_id, &attachment_id).await {
            Ok(record) => IpcResponse::ok(&record),
            Err(e) => refused(&e, "attachment info"),
        },
        PinRequest::VoiceMessageSend {
            channel_id, pcm, ..
        } => match files.voice(key, &channel_id, &pcm).await {
            Ok(id) => IpcResponse::ok(&id),
            Err(e) => refused(&e, "send voice message"),
        },
        PinRequest::VoiceMessageAudio {
            channel_id,
            attachment_id,
            ..
        } => match files.voice_audio(key, &channel_id, &attachment_id).await {
            Ok(view) => IpcResponse::ok(&view),
            Err(e) => refused(&e, "voice message audio"),
        },
        other => IpcResponse::error(
            500,
            format!("{} is not an attachment request", other.name()),
        ),
    }
}
