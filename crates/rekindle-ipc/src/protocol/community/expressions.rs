//! The custom emoji and stickers (plan E3.1), carried as `CommunityRequest::Expressions`.

use serde::{Deserialize, Serialize};

use crate::message::SecurityLevel;
use crate::protocol::{AgentCapability, Lane};

/// A custom emoji and stickers request.
#[derive(Clone, Serialize, Deserialize)]
pub enum ExpressionRequest {
    /// Upload a custom emoji (`MANAGE_EXPRESSIONS`): `kind` is what the
    /// picker declared, and the picture is decoded as that kind only.
    EmojiUpload {
        community: String,
        name: String,
        bytes: Vec<u8>,
        kind: rekindle_types::image_kind::ImageKind,
    },

    /// Upload a custom sticker (`MANAGE_EXPRESSIONS`).
    StickerUpload {
        community: String,
        name: String,
        bytes: Vec<u8>,
        kind: rekindle_types::image_kind::ImageKind,
        tags: Vec<String>,
    },

    /// Delete a custom expression (`MANAGE_EXPRESSIONS`).
    ExpressionDelete {
        community: String,
        expression_id: String,
    },

    /// The community's expressions, with the cached pictures inline.
    Expressions { community: String },
}

impl ExpressionRequest {
    /// The variant name, without its fields.
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.class().0
    }

    pub(crate) fn class(&self) -> (&'static str, Lane, SecurityLevel) {
        use Lane::{Query, Write};
        use SecurityLevel::Authenticated;
        match self {
            Self::EmojiUpload { .. } => ("EmojiUpload", Write, Authenticated),
            Self::StickerUpload { .. } => ("StickerUpload", Write, Authenticated),
            Self::ExpressionDelete { .. } => ("ExpressionDelete", Write, Authenticated),
            Self::Expressions { .. } => ("Expressions", Query, Authenticated),
        }
    }

    /// The capability an `Agent`-level sender needs for this request, which every
    /// request of this domain has.
    #[must_use]
    pub fn agent_capability(&self) -> AgentCapability {
        use AgentCapability as C;
        match self {
            Self::Expressions { .. } => C::CommunitiesRead,
            Self::EmojiUpload { .. }
            | Self::StickerUpload { .. }
            | Self::ExpressionDelete { .. } => C::CommunitiesWrite,
        }
    }
}
