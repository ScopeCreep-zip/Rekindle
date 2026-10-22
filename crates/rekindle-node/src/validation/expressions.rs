//! Expression requests (plan E3.1 E3c): the community key, a name the
//! channel pipeline accepts, a picture within its declared kind's size (the
//! pipeline then decodes it as that kind only), and an expression id of 16
//! bytes of hex.

use rekindle_channel::expressions::{
    validate_expression_name, MAX_ANIMATED_EMOJI_BYTES, MAX_STATIC_EMOJI_BYTES, MAX_STICKER_BYTES,
};
use rekindle_ipc::protocol::{CommunityRequest, IpcResponse};
use rekindle_types::image_kind::ImageKind;

use super::{community, Checked};

fn picture(name: &str, bytes: &[u8], max: usize) -> Checked {
    validate_expression_name(name).map_err(|e| IpcResponse::error(400, e.to_string()))?;
    if bytes.is_empty() || bytes.len() > max {
        return Err(IpcResponse::error(
            413,
            format!("the picture must be 1–{max} bytes, not {}", bytes.len()),
        ));
    }
    Ok(())
}

pub(super) fn check(request: &CommunityRequest) -> Checked {
    match request {
        CommunityRequest::EmojiUpload {
            community: key,
            name,
            bytes,
            kind,
        } => {
            community(key)?;
            let max = if *kind == ImageKind::Gif {
                MAX_ANIMATED_EMOJI_BYTES
            } else {
                MAX_STATIC_EMOJI_BYTES
            };
            picture(name, bytes, max)
        }
        CommunityRequest::StickerUpload {
            community: key,
            name,
            bytes,
            ..
        } => {
            community(key)?;
            picture(name, bytes, MAX_STICKER_BYTES)
        }
        CommunityRequest::ExpressionDelete {
            community: key,
            expression_id,
        } => {
            community(key)?;
            match hex::decode(expression_id) {
                Ok(bytes) if bytes.len() == 16 => Ok(()),
                _ => Err(IpcResponse::error(
                    400,
                    "invalid expression id: must be 32 hex characters",
                )),
            }
        }
        CommunityRequest::Expressions { community: key } => community(key),
        other => Err(IpcResponse::error(
            500,
            format!("{} is not an expression request", other.name()),
        )),
    }
}
