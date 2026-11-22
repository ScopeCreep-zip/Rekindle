//! The file transfers (plan E7.2 G12, SK2-G design §4), answered by the
//! Host's transfer driver (`host::transfers`) on both hosts.

use rekindle_ipc::protocol::{IpcResponse, TransferRequest};
use rekindle_types::dto::skin_run::{ShellOp, SkinRun};
use rekindle_types::dto::transfer::{TransferReveal, TransferTarget};

use super::DaemonContext;
use crate::host::adapters::files::HostFiles;
use crate::host::transfers::{self, Ended, Refused};

fn refused(r: &Refused, what: &str) -> IpcResponse {
    match r {
        Refused::Locked => IpcResponse::error(423, format!("{what}: no identity is unlocked")),
        Refused::Db(e) => IpcResponse::error(500, format!("{what}: {e}")),
    }
}

/// A finished download's answer.
pub(super) fn answer(ended: Result<Ended, Refused>, what: &str) -> IpcResponse {
    match ended {
        Ok(Ended::Done) => IpcResponse::ok(&()),
        Ok(Ended::Stopped) => IpcResponse::error(409, format!("{what}: stopped")),
        Ok(Ended::Failed(e)) => super::attachments::refused(&e, what),
        Err(r) => refused(&r, what),
    }
}

fn missing(attachment_id: &str) -> IpcResponse {
    IpcResponse::error(404, format!("no transfer {attachment_id}"))
}

/// `TransferStart`: resume a stored transfer from its checkpoint, or
/// start a never-fetched attachment — first asking the shell where to
/// save it, exactly as `download_attachment`'s save dialog does
/// (`src-tauri/src/services/community_files_runtime.rs`
/// `download_with_dialog`, `docs/architecture/files.md` "Download").
async fn start(
    ctx: &DaemonContext,
    attachment_id: String,
    target: TransferTarget,
    dest: Option<String>,
) -> IpcResponse {
    let what = "start transfer";
    let stored = match transfers::get(ctx, &attachment_id).await {
        Ok(stored) => stored,
        Err(r) => return refused(&r, what),
    };
    let (target, dest) = match (stored, dest) {
        (Some(row), None) if row.target == target => {
            if !row.state.resumable() {
                return IpcResponse::error(
                    409,
                    format!("{what}: {attachment_id} is {}", row.state.as_str()),
                );
            }
            (row.target, row.dest_path)
        }
        (Some(_), _) => {
            return IpcResponse::error(
                409,
                format!("{what}: {attachment_id} is already a transfer; resume it as stored"),
            )
        }
        (None, Some(dest)) => (target, dest),
        (None, None) => return choose_dest(ctx, attachment_id, target).await,
    };
    match transfers::download(ctx, target, &attachment_id, dest.into()).await {
        Ok(Ended::Done) => IpcResponse::ok(&None::<SkinRun>),
        ended => answer(ended, what),
    }
}

/// A never-fetched attachment's start without a destination: the shell's
/// save dialog, named after the offer's sanitized filename. 404 when no
/// message of the channel offers it.
async fn choose_dest(
    ctx: &DaemonContext,
    attachment_id: String,
    target: TransferTarget,
) -> IpcResponse {
    let TransferTarget::Channel { channel_id, .. } = &target;
    let files = HostFiles::new(super::adapter(ctx));
    match files.info(channel_id, &attachment_id).await {
        Ok(Some(record)) => IpcResponse::ok(&Some(SkinRun::Shell(ShellOp::ChooseDownloadDest {
            suggested_name: rekindle_files::sanitize_filename(&record.filename),
            attachment_id,
            target,
        }))),
        Ok(None) => IpcResponse::error(
            404,
            format!("start transfer: no message offers {attachment_id}"),
        ),
        Err(e) => super::attachments::refused(&e, "start transfer"),
    }
}

/// One transfer request.
pub(super) async fn route(ctx: &DaemonContext, request: TransferRequest) -> IpcResponse {
    match request {
        TransferRequest::TransferList => match transfers::list(ctx).await {
            Ok(rows) => IpcResponse::ok(&rows),
            Err(r) => refused(&r, "transfer list"),
        },
        TransferRequest::TransferStart {
            attachment_id,
            target,
            dest,
        } => start(ctx, attachment_id, target, dest).await,
        TransferRequest::TransferStop { attachment_id } => {
            if transfers::stop(ctx, &attachment_id) {
                IpcResponse::ok(&())
            } else {
                IpcResponse::error(
                    409,
                    format!("stop transfer: {attachment_id} is not running"),
                )
            }
        }
        TransferRequest::TransferOpen { attachment_id } => {
            match transfers::get(ctx, &attachment_id).await {
                Ok(Some(row)) => match transfers::revealable(&row) {
                    Some(path) => IpcResponse::ok(&TransferReveal { path }),
                    None => IpcResponse::error(
                        409,
                        format!("open transfer: {attachment_id} has no finished file"),
                    ),
                },
                Ok(None) => missing(&attachment_id),
                Err(r) => refused(&r, "open transfer"),
            }
        }
        TransferRequest::TransferRemove {
            attachment_id,
            delete_file,
        } => match transfers::get(ctx, &attachment_id).await {
            Ok(Some(row)) => match transfers::remove(ctx, &row, delete_file).await {
                Ok(()) => IpcResponse::ok(&()),
                Err(r) => refused(&r, "remove transfer"),
            },
            Ok(None) => missing(&attachment_id),
            Err(r) => refused(&r, "remove transfer"),
        },
    }
}
