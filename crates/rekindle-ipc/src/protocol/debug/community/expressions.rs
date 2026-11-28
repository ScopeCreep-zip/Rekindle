//! Expressions (plan E3.1 E3c): an upload shows its picture's length only.

use crate::protocol::ExpressionRequest;

impl std::fmt::Debug for ExpressionRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmojiUpload {
                community,
                name,
                bytes,
                kind,
            } => f
                .debug_struct("EmojiUpload")
                .field("community", community)
                .field("name", name)
                .field("bytes", &bytes.len())
                .field("kind", kind)
                .finish(),
            Self::StickerUpload {
                community,
                name,
                bytes,
                kind,
                tags,
            } => f
                .debug_struct("StickerUpload")
                .field("community", community)
                .field("name", name)
                .field("bytes", &bytes.len())
                .field("kind", kind)
                .field("tags", &tags.len())
                .finish(),
            Self::ExpressionDelete {
                community,
                expression_id,
            } => f
                .debug_struct("ExpressionDelete")
                .field("community", community)
                .field("expression_id", expression_id)
                .finish(),
            Self::Expressions { community } => f
                .debug_struct("Expressions")
                .field("community", community)
                .finish(),
        }
    }
}
