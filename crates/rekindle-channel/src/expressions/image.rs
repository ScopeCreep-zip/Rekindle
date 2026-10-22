//! Emoji and sticker pictures through `rekindle-image` (plan E3.1, E7.11).
//!
//! An upload is decoded by the kind the file picker declared (PNG, JPEG or
//! GIF; never WebP), under limits, scaled to the expression's size and
//! re-encoded: a still as PNG, an animated GIF as GIF. Nothing of the
//! source but its pixels (and a GIF's frame delays) survives, so EXIF, ICC
//! and text chunks never reach the community.
//!
//! What a peer published is decoded again before a `data:` URL is built
//! from it: the kind an expression's `animated` flag implies (PNG for a
//! still, GIF for an animation), the bytes' magic must match it, and its
//! size must be one a conforming upload produces. Anything else is not
//! rendered.

use rekindle_image::{
    decode_limited, encode_gif, encode_png, fit_within, DecodeLimits, Decoded, DynamicImage, Frame,
    Frames, ImageKind,
};

use super::limits::{MAX_ANIMATED_EMOJI_BYTES, MAX_STATIC_EMOJI_BYTES, MAX_STICKER_BYTES};
use crate::error::ChannelError;

/// The longest side an emoji is shown at, px.
pub const EMOJI_MAX_SIDE: u32 = 128;
/// The longest side a sticker is shown at, px.
pub const STICKER_MAX_SIDE: u32 = 320;
/// The most frames an animated emoji or sticker has.
pub const MAX_EXPRESSION_FRAMES: u32 = 120;

/// The largest source picture an upload decodes.
const SOURCE_LIMITS: DecodeLimits = DecodeLimits {
    max_width: 4096,
    max_height: 4096,
    max_pixels: 16_777_216,
    max_alloc: 128 * 1024 * 1024,
    frames: Frames::First,
};

/// The most an animated source's frames hold together.
const ANIMATION_MAX_ALLOC: u64 = 128 * 1024 * 1024;

/// Which expression a picture is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExpressionPicture {
    Emoji,
    Sticker,
}

impl ExpressionPicture {
    const fn label(self) -> &'static str {
        match self {
            Self::Emoji => "emoji",
            Self::Sticker => "sticker",
        }
    }

    const fn max_side(self) -> u32 {
        match self {
            Self::Emoji => EMOJI_MAX_SIDE,
            Self::Sticker => STICKER_MAX_SIDE,
        }
    }

    /// The most bytes the picture (source or re-encoded) may be.
    const fn max_bytes(self, animated: bool) -> usize {
        match (self, animated) {
            (Self::Emoji, false) => MAX_STATIC_EMOJI_BYTES,
            (Self::Emoji, true) => MAX_ANIMATED_EMOJI_BYTES,
            (Self::Sticker, _) => MAX_STICKER_BYTES,
        }
    }

    /// The limits a published picture is decoded under: no side past
    /// what an upload is scaled to.
    const fn published_limits(self, animated: bool) -> DecodeLimits {
        let side = self.max_side();
        DecodeLimits {
            max_width: side,
            max_height: side,
            max_pixels: side as u64 * side as u64,
            max_alloc: ANIMATION_MAX_ALLOC,
            frames: if animated {
                Frames::AtMost(MAX_EXPRESSION_FRAMES)
            } else {
                Frames::First
            },
        }
    }
}

/// A re-encoded upload.
#[derive(Debug)]
pub struct PreparedPicture {
    /// The metadata-free bytes to publish.
    pub bytes: Vec<u8>,
    /// Their kind: PNG for a still, GIF for an animation.
    pub kind: ImageKind,
}

impl PreparedPicture {
    /// Whether the picture is animated (a GIF).
    #[must_use]
    pub fn animated(&self) -> bool {
        self.kind == ImageKind::Gif
    }

    /// The file extension the cache stores it under.
    #[must_use]
    pub fn extension(&self) -> &'static str {
        if self.animated() {
            "gif"
        } else {
            "png"
        }
    }
}

/// Decode `bytes` as `declared` under limits, scale it to `target`'s size
/// and re-encode it without metadata. A GIF stays animated; PNG and JPEG
/// become a still PNG.
///
/// # Errors
/// [`ChannelError::InvalidId`] when the bytes are empty, not `declared`,
/// over a limit or undecodable; [`ChannelError::BodyTooLarge`] when the
/// source or the re-encoded picture is over the byte cap.
pub fn prepare_upload(
    bytes: &[u8],
    declared: ImageKind,
    target: ExpressionPicture,
) -> Result<PreparedPicture, ChannelError> {
    let animated = declared == ImageKind::Gif;
    let max = target.max_bytes(animated);
    if bytes.is_empty() {
        return Err(ChannelError::InvalidId(format!(
            "{} upload cannot be empty",
            target.label()
        )));
    }
    if bytes.len() > max {
        return Err(ChannelError::BodyTooLarge {
            size: bytes.len(),
            max,
        });
    }
    let limits = DecodeLimits {
        frames: if animated {
            Frames::AtMost(MAX_EXPRESSION_FRAMES)
        } else {
            Frames::First
        },
        max_alloc: if animated {
            ANIMATION_MAX_ALLOC
        } else {
            SOURCE_LIMITS.max_alloc
        },
        ..SOURCE_LIMITS
    };
    let refused = |e: rekindle_image::ImageError| {
        ChannelError::InvalidId(format!("{} picture refused: {e}", target.label()))
    };
    let side = target.max_side();
    let (out, kind) = match decode_limited(bytes, declared, &limits).map_err(refused)? {
        // PNG and JPEG (a GIF decodes with `AtMost`, so as an animation).
        Decoded::Still(picture) => (
            encode_png(&fit_within(picture, side)).map_err(refused)?,
            ImageKind::Png,
        ),
        Decoded::Animated { first, rest } => {
            let frames = std::iter::once(first)
                .chain(rest)
                .map(|frame| scaled_frame(frame, side))
                .collect();
            (encode_gif(frames).map_err(refused)?, ImageKind::Gif)
        }
    };
    if out.len() > max {
        return Err(ChannelError::BodyTooLarge {
            size: out.len(),
            max,
        });
    }
    Ok(PreparedPicture { bytes: out, kind })
}

/// One composited frame scaled so no side exceeds `side`, its delay kept.
fn scaled_frame(frame: Frame, side: u32) -> Frame {
    let delay = frame.delay();
    let picture = fit_within(DynamicImage::ImageRgba8(frame.into_buffer()), side);
    Frame::from_parts(picture.into_rgba8(), 0, 0, delay)
}

/// The MIME type to render a published picture under, or `None` when it
/// is not one a conforming upload produces: over the byte cap, not the
/// kind `animated` implies, too large, or undecodable.
#[must_use]
pub fn published_media_type(
    bytes: &[u8],
    animated: bool,
    target: ExpressionPicture,
) -> Option<&'static str> {
    if bytes.is_empty() || bytes.len() > target.max_bytes(animated) {
        return None;
    }
    let kind = if animated {
        ImageKind::Gif
    } else {
        ImageKind::Png
    };
    decode_limited(bytes, kind, &target.published_limits(animated))
        .ok()
        .map(|_| kind.mime())
}

#[cfg(test)]
mod tests;
