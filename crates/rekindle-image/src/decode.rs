//! [`decode_limited`]: the declared kind's decoder only, under limits.

use std::io::Cursor;

use image::codecs::gif::GifDecoder;
use image::error::LimitErrorKind;
use image::{AnimationDecoder, DynamicImage, Frame, ImageDecoder, ImageFormat, ImageReader};
use rekindle_types::image_kind::ImageKind;

use crate::error::{ImageError, TooLarge};
use crate::kind::{has_magic, kind_of};

/// How many frames of a GIF to read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frames {
    /// The first frame only: the picture a still renderer shows (an
    /// avatar). Any GIF, animated or not, decodes to [`Decoded::Still`].
    First,
    /// Every frame, refused past this many (a skin's animation). A GIF
    /// decodes to [`Decoded::Animated`]; PNG and JPEG stay still.
    AtMost(u32),
}

/// What one decode may cost. The sides and pixel count are checked against
/// the header before any pixel buffer is allocated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecodeLimits {
    /// The widest image taken, px.
    pub max_width: u32,
    /// The tallest image taken, px.
    pub max_height: u32,
    /// The most pixels (width × height) taken.
    pub max_pixels: u64,
    /// The most bytes the decode may hold at once: `image::Limits::max_alloc`,
    /// and the sum of an animation's frames.
    pub max_alloc: u64,
    /// How many GIF frames to read.
    pub frames: Frames,
}

impl DecodeLimits {
    /// The header's size against the side and pixel limits.
    fn check(&self, width: u32, height: u32) -> Result<(), TooLarge> {
        if width > self.max_width || height > self.max_height {
            return Err(TooLarge::Side {
                width,
                height,
                max_width: self.max_width,
                max_height: self.max_height,
            });
        }
        let pixels = u64::from(width) * u64::from(height);
        if pixels > self.max_pixels {
            return Err(TooLarge::Pixels {
                pixels,
                max_pixels: self.max_pixels,
            });
        }
        Ok(())
    }

    /// The same limits as `image` takes them.
    fn image_limits(&self) -> image::Limits {
        let mut limits = image::Limits::no_limits();
        limits.max_image_width = Some(self.max_width);
        limits.max_image_height = Some(self.max_height);
        limits.max_alloc = Some(self.max_alloc);
        limits
    }
}

/// A decoded image.
pub enum Decoded {
    /// One picture: a PNG, a JPEG, or a GIF read with [`Frames::First`].
    Still(DynamicImage),
    /// A GIF read with [`Frames::AtMost`], each frame composited onto the
    /// full canvas.
    Animated {
        /// The first frame.
        first: Frame,
        /// The frames after it, in order.
        rest: Vec<Frame>,
    },
}

impl Decoded {
    /// The picture a still renderer shows: the image, or the first frame.
    #[must_use]
    pub fn into_first(self) -> DynamicImage {
        match self {
            Self::Still(image) => image,
            Self::Animated { first, .. } => DynamicImage::ImageRgba8(first.into_buffer()),
        }
    }
}

/// Decode `bytes`, which the caller declares to be `declared`.
///
/// The magic bytes must be `declared`'s, and only `declared`'s decoder
/// runs. The header is read and its size checked against `limits` before
/// any pixel buffer is allocated; the decode itself runs under
/// `image::Limits` built from `limits`.
///
/// # Errors
/// [`ImageError::KindMismatch`] when the bytes are another kind (or none),
/// [`ImageError::TooLarge`] when a limit is exceeded, and
/// [`ImageError::Decode`] when the decoder refuses the bytes.
pub fn decode_limited(
    bytes: &[u8],
    declared: ImageKind,
    limits: &DecodeLimits,
) -> Result<Decoded, ImageError> {
    if !has_magic(bytes, declared) {
        return Err(ImageError::KindMismatch {
            declared,
            found: kind_of(bytes),
        });
    }
    let format = match declared {
        ImageKind::Png => ImageFormat::Png,
        ImageKind::Jpeg => ImageFormat::Jpeg,
        ImageKind::Gif => ImageFormat::Gif,
    };
    let (width, height) = header_dimensions(bytes, format, limits)?;
    limits.check(width, height)?;
    if let (ImageKind::Gif, Frames::AtMost(max_frames)) = (declared, limits.frames) {
        return animation(bytes, (width, height), limits, max_frames);
    }
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits.image_limits());
    reader
        .decode()
        .map(Decoded::Still)
        .map_err(|e| refused(e, limits))
}

/// The header's width and height. The side limits are left to
/// [`DecodeLimits::check`] so its error names the size; only `max_alloc`
/// bounds what reading the header holds.
fn header_dimensions(
    bytes: &[u8],
    format: ImageFormat,
    limits: &DecodeLimits,
) -> Result<(u32, u32), ImageError> {
    let mut header_limits = image::Limits::no_limits();
    header_limits.max_alloc = Some(limits.max_alloc);
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(header_limits);
    reader.into_dimensions().map_err(|e| refused(e, limits))
}

/// Every frame of a GIF whose canvas is `canvas`: at most `max_frames`, and
/// their buffers together within `max_alloc`.
fn animation(
    bytes: &[u8],
    canvas: (u32, u32),
    limits: &DecodeLimits,
    max_frames: u32,
) -> Result<Decoded, ImageError> {
    let mut decoder = GifDecoder::new(Cursor::new(bytes)).map_err(|e| refused(e, limits))?;
    decoder
        .set_limits(limits.image_limits())
        .map_err(|e| refused(e, limits))?;
    // Each frame is an RGBA8 buffer the size of the canvas.
    let frame_bytes = u64::from(canvas.0) * u64::from(canvas.1) * 4;
    let mut held = 0u64;
    let mut count = 0u32;
    let mut first = None;
    let mut rest = Vec::new();
    for frame in decoder.into_frames() {
        let frame = frame.map_err(|e| refused(e, limits))?;
        count = count.saturating_add(1);
        if count > max_frames {
            return Err(TooLarge::Frames { max_frames }.into());
        }
        held = held.saturating_add(frame_bytes);
        if held > limits.max_alloc {
            return Err(TooLarge::Allocation {
                max_alloc: limits.max_alloc,
            }
            .into());
        }
        if first.is_none() {
            first = Some(frame);
        } else {
            rest.push(frame);
        }
    }
    let first = first.ok_or_else(|| ImageError::Decode("a GIF without frames".into()))?;
    Ok(Decoded::Animated { first, rest })
}

/// `image`'s refusal as ours: a limit it enforced is [`ImageError::TooLarge`].
fn refused(error: image::ImageError, limits: &DecodeLimits) -> ImageError {
    match error {
        image::ImageError::Limits(limit) => match limit.kind() {
            LimitErrorKind::InsufficientMemory => TooLarge::Allocation {
                max_alloc: limits.max_alloc,
            }
            .into(),
            LimitErrorKind::DimensionError => TooLarge::FrameSide {
                max_width: limits.max_width,
                max_height: limits.max_height,
            }
            .into(),
            _ => ImageError::Decode(limit.to_string()),
        },
        other => ImageError::Decode(other.to_string()),
    }
}
