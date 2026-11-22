//! The file transfers (plan E7.2 G12, SK2-G design §4), answered by the
//! Host's transfer driver (`host::transfers`) on both hosts.

use rekindle_ipc::protocol::{IpcResponse, TransferRequest};
use rekindle_types::dto::transfer::TransferReveal;

use super::DaemonContext;
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

/// One transfer request.
pub(super) async fn route(ctx: &DaemonContext, request: TransferRequest) -> IpcResponse {
    match request {
        TransferRequest::TransferList => match transfers::list(ctx).await {
            Ok(rows) => IpcResponse::ok(&rows),
            Err(r) => refused(&r, "transfer list"),
        },
        TransferRequest::TransferStart { attachment_id } => {
            let row = match transfers::get(ctx, &attachment_id).await {
                Ok(Some(row)) => row,
                Ok(None) => return missing(&attachment_id),
                Err(r) => return refused(&r, "start transfer"),
            };
            if !row.state.resumable() {
                return IpcResponse::error(
                    409,
                    format!("start transfer: {attachment_id} is {}", row.state.as_str()),
                );
            }
            answer(
                transfers::download(ctx, row.target, &attachment_id, row.dest_path.into()).await,
                "start transfer",
            )
        }
        TransferRequest::TransferStop { attachment_id } => {
            if transfers::stop(ctx, &attachment_id).await {
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
