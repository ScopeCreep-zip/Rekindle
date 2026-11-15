//! Validator step 4: images (plan E7.7.1).
//!
//! Every file under `assets/` and `icons/` is a picture, PNG or GIF only.
//! Its declared type is its extension; the bytes' magic must match it and
//! only that type's decoder runs (the format is never sniffed: the
//! FORCEDENTRY path). WebP (CVE-2023-4863), JPEG, BMP, TIFF, AVIF, APNG and
//! SVG are refused by name, and a PNG carrying an `acTL` chunk (an APNG
//! under a `.png` name) is refused too.
//!
//! Every header is read first, and each image's sides and pixels and the
//! sum over all images checked, before any pixel is decoded. Each image is
//! then decoded through `rekindle-image` under limits (`image::Limits`
//! with `max_alloc`, which reaches the `png` crate as `png::Limits`, and the
//! GIF frame counter, each frame past the first counted against the total)
//! and re-encoded: a PNG to an RGBA8 PNG, a GIF to a looping GIF of its
//! composited frames, after the flash test ([`crate::flash`], WCAG 2.3.1:
//! at most three flashes a second). No chunk, extension, comment, EXIF or ICC profile of
//! the source survives (Chromium's `ImageSanitizer`).
//!
//! The manifest's region images, state images and masks must name files
//! under `assets/`, and its icons files under `icons/`, that the package
//! holds.

use std::collections::BTreeMap;

use rekindle_image::{
    decode_limited, encode_gif, encode_png, read_header, DecodeLimits, Decoded, DynamicImage,
    Frames, ImageError, ImageKind, TooLarge,
};
use rekindle_types::dto::skin::{SkinError, SkinManifest, SkinRule};

use crate::flash::{self, MAX_FLASHES_PER_SECOND};
use crate::limits::{
    MAX_IMAGE_ALLOC, MAX_IMAGE_FRAMES, MAX_IMAGE_PIXELS, MAX_IMAGE_SIDE, MAX_TOTAL_IMAGE_PIXELS,
};
use crate::package::{Package, MANIFEST};

/// The directories whose files are images.
pub const IMAGE_DIRS: [&str; 2] = ["assets/", "icons/"];

/// Extensions of the formats a skin may not carry, and the format's name.
const REFUSED: [(&str, &str); 13] = [
    ("webp", "WebP"),
    ("jpg", "JPEG"),
    ("jpeg", "JPEG"),
    ("jpe", "JPEG"),
    ("jfif", "JPEG"),
    ("bmp", "BMP"),
    ("dib", "BMP"),
    ("tif", "TIFF"),
    ("tiff", "TIFF"),
    ("avif", "AVIF"),
    ("apng", "APNG"),
    ("svg", "SVG"),
    ("svgz", "SVG"),
];

/// The file name's extension, lowercased.
pub(crate) fn extension(name: &str) -> Option<String> {
    let file = name.rsplit_once('/').map_or(name, |(_, file)| file);
    file.rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
}

/// The type an image entry declares by its extension.
fn declared_kind(name: &str) -> Result<ImageKind, SkinRule> {
    let ext = extension(name);
    match ext.as_deref() {
        Some("png") => return Ok(ImageKind::Png),
        Some("gif") => return Ok(ImageKind::Gif),
        _ => {}
    }
    let refused = REFUSED.iter().find(|(e, _)| ext.as_deref() == Some(*e));
    Err(match refused {
        Some((_, format)) => SkinRule::RefusedImageFormat {
            format: (*format).to_owned(),
        },
        None => SkinRule::UnsupportedEntryType {
            expected: "a .png or .gif image".to_owned(),
        },
    })
}

/// Checks every image entry and returns each one's canonical bytes, by
/// entry name.
///
/// # Errors
/// The first rule an image or a manifest reference breaks.
pub fn validate(
    package: &Package<'_>,
    manifest: &SkinManifest,
) -> Result<BTreeMap<String, Vec<u8>>, SkinError> {
    check_references(package, manifest)?;
    let mut checked = Vec::new();
    let mut total = 0u64;
    for (name, bytes) in package.files() {
        if !IMAGE_DIRS.iter().any(|dir| name.starts_with(dir)) {
            continue;
        }
        let refuse = |rule| SkinError::new(name, rule);
        let kind = declared_kind(name).map_err(refuse)?;
        let header = read_header(bytes, kind, MAX_IMAGE_ALLOC).map_err(|e| refuse(rule_of(e)))?;
        if header.animated_png {
            return Err(refuse(SkinRule::AnimatedPng));
        }
        if header.width > MAX_IMAGE_SIDE || header.height > MAX_IMAGE_SIDE {
            return Err(refuse(SkinRule::ImageSide {
                width: header.width,
                height: header.height,
                max: MAX_IMAGE_SIDE,
            }));
        }
        let pixels = header.pixels();
        if pixels > MAX_IMAGE_PIXELS {
            return Err(refuse(SkinRule::ImagePixels {
                pixels,
                max: MAX_IMAGE_PIXELS,
            }));
        }
        total = total.saturating_add(pixels);
        if total > MAX_TOTAL_IMAGE_PIXELS {
            return Err(refuse(SkinRule::ImagesTotalPixels {
                pixels: total,
                max: MAX_TOTAL_IMAGE_PIXELS,
            }));
        }
        checked.push((name, bytes, kind, pixels));
    }
    let mut budget = Budget { total };
    let mut out = BTreeMap::new();
    for (name, bytes, kind, pixels) in checked {
        let canonical = match kind {
            ImageKind::Gif => canonical_gif(bytes, pixels, &mut budget),
            _ => canonical_png(bytes),
        }
        .map_err(|rule| SkinError::new(name, rule))?;
        out.insert(name.to_owned(), canonical);
    }
    Ok(out)
}

/// The pixels counted so far: every header's, plus each animation's frames
/// past the first.
struct Budget {
    total: u64,
}

fn still_limits(frames: Frames, max_alloc: u64) -> DecodeLimits {
    DecodeLimits {
        max_width: MAX_IMAGE_SIDE,
        max_height: MAX_IMAGE_SIDE,
        max_pixels: MAX_IMAGE_PIXELS,
        max_alloc,
        frames,
    }
}

/// A PNG's pixels as an RGBA8 PNG.
fn canonical_png(bytes: &[u8]) -> Result<Vec<u8>, SkinRule> {
    let limits = still_limits(Frames::First, MAX_IMAGE_ALLOC);
    let image = decode_limited(bytes, ImageKind::Png, &limits)
        .map_err(rule_of)?
        .into_first();
    encode_png(&DynamicImage::ImageRgba8(image.into_rgba8())).map_err(rule_of)
}

/// A GIF's composited frames as a looping GIF. The frames past the first
/// may take only what is left of the total, so the frame limit is the
/// smaller of [`MAX_IMAGE_FRAMES`] and what fits.
fn canonical_gif(bytes: &[u8], pixels: u64, budget: &mut Budget) -> Result<Vec<u8>, SkinRule> {
    let pixels = pixels.max(1);
    let left = MAX_TOTAL_IMAGE_PIXELS - budget.total;
    let fits = u32::try_from(1 + left / pixels).unwrap_or(u32::MAX);
    let max_frames = fits.min(MAX_IMAGE_FRAMES);
    let max_alloc = u64::from(max_frames) * pixels * 4;
    let limits = still_limits(Frames::AtMost(max_frames), max_alloc.max(MAX_IMAGE_ALLOC));
    let decoded = decode_limited(bytes, ImageKind::Gif, &limits).map_err(|e| match e {
        ImageError::TooLarge(TooLarge::Frames { .. }) if max_frames < MAX_IMAGE_FRAMES => {
            SkinRule::ImagesTotalPixels {
                pixels: budget.total + u64::from(max_frames) * pixels,
                max: MAX_TOTAL_IMAGE_PIXELS,
            }
        }
        other => rule_of(other),
    })?;
    let frames = match decoded {
        Decoded::Animated { first, rest } => {
            let mut frames = Vec::with_capacity(rest.len() + 1);
            frames.push(first);
            frames.extend(rest);
            frames
        }
        Decoded::Still(_) => {
            return Err(SkinRule::ImageDecode {
                detail: "a GIF read as a still".to_owned(),
            })
        }
    };
    let extra = u64::try_from(frames.len() - 1).unwrap_or(u64::MAX);
    budget.total = budget.total.saturating_add(extra.saturating_mul(pixels));
    let flashes = flash::max_flashes(&frames);
    if flashes > MAX_FLASHES_PER_SECOND {
        return Err(SkinRule::Flashing {
            flashes,
            max: MAX_FLASHES_PER_SECOND,
        });
    }
    encode_gif(frames).map_err(rule_of)
}

/// `rekindle-image`'s refusal as a skin rule.
fn rule_of(error: ImageError) -> SkinRule {
    match error {
        ImageError::KindMismatch { declared, found } => SkinRule::ImageKindMismatch {
            declared: declared.mime().to_owned(),
            found: found
                .map_or("no known image kind", ImageKind::mime)
                .to_owned(),
        },
        ImageError::TooLarge(limit) => match limit {
            TooLarge::Side { width, height, .. } => SkinRule::ImageSide {
                width,
                height,
                max: MAX_IMAGE_SIDE,
            },
            TooLarge::Pixels { pixels, max_pixels } => SkinRule::ImagePixels {
                pixels,
                max: max_pixels,
            },
            TooLarge::FrameSide { .. } => SkinRule::ImageFrameSide {
                max: MAX_IMAGE_SIDE,
            },
            TooLarge::Frames { max_frames } => SkinRule::TooManyFrames { max: max_frames },
            TooLarge::Allocation { max_alloc } => SkinRule::ImageAllocation { max: max_alloc },
        },
        ImageError::Decode(detail) => SkinRule::ImageDecode { detail },
        ImageError::Encode(detail) => SkinRule::ImageEncode { detail },
    }
}

/// The manifest's image references: each names a file the package holds,
/// under `dir`.
fn check_reference(
    package: &Package<'_>,
    field: &str,
    path: &str,
    dir: &str,
) -> Result<(), SkinError> {
    if !path.starts_with(dir) {
        return Err(SkinError::new(
            MANIFEST,
            SkinRule::BadValue {
                field: field.to_owned(),
                expected: format!("an entry under {dir}"),
            },
        ));
    }
    if package.get(path).is_none() {
        return Err(SkinError::new(
            path,
            SkinRule::MissingEntry {
                field: field.to_owned(),
            },
        ));
    }
    Ok(())
}

fn check_references(package: &Package<'_>, manifest: &SkinManifest) -> Result<(), SkinError> {
    for (id, region) in &manifest.regions {
        check_reference(
            package,
            &format!("regions.{id}.image"),
            &region.image,
            "assets/",
        )?;
        for (state, path) in &region.states {
            let field = format!("regions.{id}.states.{state:?}");
            check_reference(package, &field, path, "assets/")?;
        }
        if let Some(mask) = &region.mask {
            check_reference(package, &format!("regions.{id}.mask"), mask, "assets/")?;
        }
    }
    for (id, path) in &manifest.icons {
        check_reference(package, &format!("icons.{id}"), path, "icons/")?;
    }
    Ok(())
}
