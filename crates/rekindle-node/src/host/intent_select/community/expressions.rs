//! The selection of a expression request (plans D3.3, E3.1).

use super::{community, DaemonContext, ExpressionRequest, IpcResponse, Selection, Subject};

/// `selection` for the expression requests.
pub(super) fn selection(
    ctx: &DaemonContext,
    request: &ExpressionRequest,
    s: &mut Selection,
) -> Result<(), IpcResponse> {
    match request {
        ExpressionRequest::EmojiUpload {
            community: target, ..
        }
        | ExpressionRequest::StickerUpload {
            community: target, ..
        }
        | ExpressionRequest::ExpressionDelete {
            community: target, ..
        } => {
            let (id, name) = community(ctx, target)?;

            s.subject = Subject::Community { id, name };
        }
        ExpressionRequest::Expressions { .. } => {}
    }
    Ok(())
}
