//! A community's custom emoji and stickers (plan E3.1 E3c), on both hosts:
//! the channel pipeline (`rekindle_channel::expressions`) over the Host's
//! channel adapter, whose pictures live in the Host's chunk caches.

use rekindle_channel::deps::ExpressionView;
use rekindle_channel::error::ChannelError;
use rekindle_ipc::protocol::ExpressionRequest;
use rekindle_ipc::protocol::IpcResponse;
use rekindle_types::dto::community::ExpressionInfo;

use super::{state_error, DaemonContext};
use crate::daemon::DaemonState;
use crate::host::adapters::channel::HostChannel;

fn refused(e: &ChannelError, what: &str) -> IpcResponse {
    let code = match e {
        ChannelError::PermissionDenied(_) => 403,
        ChannelError::CommunityNotFound(_) => 404,
        ChannelError::BodyTooLarge { .. } => 413,
        ChannelError::InvalidId(_) => 400,
        _ => 500,
    };
    IpcResponse::error(code, format!("{what}: {e}"))
}

fn info(view: ExpressionView) -> ExpressionInfo {
    ExpressionInfo {
        expression_id: view.expression_id,
        name: view.name,
        kind: view.kind,
        content_hash: view.content_hash,
        inline_data_url: view.inline_data_url,
        media_type: view.media_type,
        animated: view.animated,
        tags: view.tags,
        sound_meta: view.sound_meta,
        creator_pseudonym: view.creator_pseudonym,
        created_at: view.created_at,
        available_to_peers: view.available_to_peers,
    }
}

/// An expression request.
pub(crate) async fn handle(
    ctx: &DaemonContext,
    state: DaemonState,
    request: ExpressionRequest,
) -> IpcResponse {
    let read = matches!(request, ExpressionRequest::Expressions { .. });
    if read && !state.can_query() {
        return state_error(state, "query");
    }
    if !read && !state.can_write() {
        return state_error(state, "write");
    }
    let community = match &request {
        ExpressionRequest::EmojiUpload { community, .. }
        | ExpressionRequest::StickerUpload { community, .. }
        | ExpressionRequest::ExpressionDelete { community, .. }
        | ExpressionRequest::Expressions { community } => community.clone(),
    };
    let key = match ctx.resolve_community(&community) {
        Ok(m) => m.governance_key,
        Err(e) => return e,
    };
    let channel = HostChannel::new(ctx);
    match request {
        ExpressionRequest::EmojiUpload {
            name, bytes, kind, ..
        } => match rekindle_channel::upload_emoji(&channel, &key, &name, bytes, kind).await {
            Ok(id) => IpcResponse::ok(&id),
            Err(e) => refused(&e, "upload emoji"),
        },
        ExpressionRequest::StickerUpload {
            name,
            bytes,
            kind,
            tags,
            ..
        } => {
            match rekindle_channel::upload_sticker(&channel, &key, &name, bytes, kind, tags).await {
                Ok(id) => IpcResponse::ok(&id),
                Err(e) => refused(&e, "upload sticker"),
            }
        }
        ExpressionRequest::ExpressionDelete { expression_id, .. } => {
            match rekindle_channel::delete_expression(&channel, &key, &expression_id).await {
                Ok(()) => IpcResponse::ok(&()),
                Err(e) => refused(&e, "delete expression"),
            }
        }
        ExpressionRequest::Expressions { .. } => {
            match rekindle_channel::list_expressions(&channel, &key) {
                Ok(views) => IpcResponse::ok(&views.into_iter().map(info).collect::<Vec<_>>()),
                Err(e) => refused(&e, "expressions"),
            }
        }
    }
}
