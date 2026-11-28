//! What decoding or encoding an image can fail with.

use rekindle_types::image_kind::ImageKind;

/// Why an image was refused.
#[derive(Debug, thiserror::Error)]
pub enum ImageError {
    /// The bytes are not the kind the caller declared.
    #[error("declared {declared}, but the bytes are {}", found.map_or("no known image kind", ImageKind::mime))]
    KindMismatch {
        /// What the caller said it is.
        declared: ImageKind,
        /// What the magic bytes say, if they name a kind.
        found: Option<ImageKind>,
    },
    /// The image is over a limit.
    #[error("image too large: {0}")]
    TooLarge(TooLarge),
    /// The declared decoder refused the bytes.
    #[error("image decode: {0}")]
    Decode(String),
    /// The encoder refused the picture.
    #[error("image encode: {0}")]
    Encode(String),
}

/// Which limit an image is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TooLarge {
    /// A side is longer than the limit allows (from the header).
    #[error("{width} × {height} px; at most {max_width} × {max_height}")]
    Side {
        /// The header's width.
        width: u32,
        /// The header's height.
        height: u32,
        /// The widest allowed.
        max_width: u32,
        /// The tallest allowed.
        max_height: u32,
    },
    /// More pixels than the limit allows (from the header).
    #[error("{pixels} pixels; at most {max_pixels}")]
    Pixels {
        /// Width × height.
        pixels: u64,
        /// The most allowed.
        max_pixels: u64,
    },
    /// A GIF frame reaches past the side limits (found while decoding).
    #[error("a frame is larger than {max_width} × {max_height} px")]
    FrameSide {
        /// The widest allowed.
        max_width: u32,
        /// The tallest allowed.
        max_height: u32,
    },
    /// More frames than the limit allows.
    #[error("more than {max_frames} frames")]
    Frames {
        /// The most allowed.
        max_frames: u32,
    },
    /// Decoding would allocate more than `max_alloc`.
    #[error("decoding needs more than {max_alloc} bytes")]
    Allocation {
        /// The most a decode may allocate.
        max_alloc: u64,
    },
}

impl From<TooLarge> for ImageError {
    fn from(limit: TooLarge) -> Self {
        Self::TooLarge(limit)
    }
}
