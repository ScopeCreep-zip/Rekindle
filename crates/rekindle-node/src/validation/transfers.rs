//! Transfers (plan E7.2 G12): the attachment id, and a start's place and
//! destination, checked before any operation runs.

use rekindle_ipc::protocol::{IpcResponse, TransferRequest};

use super::Checked;

/// A transfer request: its attachment id, and a start's place and
/// destination (a file path) within their bounds.
pub(super) fn check(request: &TransferRequest) -> Checked {
    request.attachment_id().map_or(Ok(()), attachment_id)?;
    if let TransferRequest::Start { target, dest, .. } = request {
        if !crate::host::skin::ids::target_fits(target) {
            return Err(IpcResponse::error(400, "transfer place out of bounds"));
        }
        if dest
            .as_ref()
            .is_some_and(|d| d.is_empty() || d.len() > 4096)
        {
            return Err(IpcResponse::error(
                400,
                "transfer destination: 1..=4096 bytes",
            ));
        }
    }
    Ok(())
}

/// An attachment id: 32 lowercase hex digits (a UUID's bytes).
fn attachment_id(s: &str) -> Checked {
    if s.len() == 32 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        Ok(())
    } else {
        Err(IpcResponse::error(
            400,
            "invalid attachment id: 32 lowercase hex digits",
        ))
    }
}
