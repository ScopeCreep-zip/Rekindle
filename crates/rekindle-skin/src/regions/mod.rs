//! Region slots and validator step 6's text over bitmaps (plan E7.7.1;
//! WCAG 2.2 F83, "Failure of Contrast (Minimum) due to using background
//! images that do not provide sufficient contrast with foreground text",
//! `.claude/plans/skins/raw/d1/wcag-F83.txt:13`).
//!
//! A region is a registry slot: an app-owned box a skin may paint a bitmap
//! into. A slot that carries text names its text tokens; the skin must give
//! it a `textBacking` colour token, painted between the image and the text.
//! For every such region, each of its images (the normal one and every
//! state's), every frame, is rendered into the slot's box at the minimum
//! and the maximum size of every window the slot appears in, and the text
//! is measured against `textBacking` composited over the worst rendered
//! pixel: ≥ 4.5:1, ≥ 3:1 for large text (1.4.3).
//!
//! * **Rendering.** Region images are drawn with `image-rendering:
//!   pixelated` (nearest neighbour), so a rendered pixel is a source pixel
//!   and the sampled set is exact. Each axis is independent: `stretch`
//!   scales the image to the box, `tile` repeats it from the top left, and
//!   `nine-slice` keeps the insets at 1:1 and stretches the middle; when the
//!   box is smaller than the two insets they shrink in proportion (CSS
//!   Backgrounds 3 `border-image-width`).
//! * **The stack under text.** Our windows are transparent, so what lies
//!   under a translucent image pixel is the desktop: it is measured over
//!   both black and white. The region's optional tint is drawn over the
//!   image, `textBacking` over that, the text over all.
//! * **Window sizes** are `src-tauri/src/windows/mod.rs:54-72` (minimum)
//!   and `:107` (the buddy list's maximum). A window without a maximum is
//!   measured at [`MAX_IMAGE_SIDE`]: once a box side reaches the image's
//!   side every source pixel on that axis is drawn, and no image is wider.

use std::collections::BTreeMap;

use rekindle_image::{decode_limited, DecodeLimits, Decoded, Frames, ImageKind, RgbaImage};
use rekindle_types::dto::skin::{
    Insets, RegionFit, RegionSpec, SkinError, SkinManifest, SkinRule, TokenValue,
};

use crate::access::scaled;
use crate::colour;
use crate::images::extension;
use crate::limits::{MAX_IMAGE_FRAMES, MAX_IMAGE_PIXELS, MAX_IMAGE_SIDE};
use crate::package::MANIFEST;
use crate::registry::{TokenEntry, TokenType};

pub mod css;
mod slots;

pub use slots::{slot_of, Extent, RegionSlot, RegionText, WindowSize, SLOTS, WINDOWS};

/// How one axis of a region image fills its box.
#[derive(Debug, Clone, Copy)]
enum Axis {
    Stretch,
    Tile,
    Slice(u32, u32),
}

/// Source index of output index `x` when `src` pixels are stretched over
/// `out` (nearest neighbour, pixel centres).
fn nearest(x: u32, out: u32, src: u32) -> u32 {
    let i = (u64::from(x) * 2 + 1) * u64::from(src) / (u64::from(out) * 2);
    u32::try_from(i).unwrap_or(u32::MAX).min(src - 1)
}

/// The distinct source indices an axis of `src` pixels draws into `out`.
fn sampled(axis: Axis, out: u32, src: u32) -> Vec<u32> {
    let mut v: Vec<u32> = match axis {
        Axis::Stretch => (0..out).map(|x| nearest(x, out, src)).collect(),
        // CSS Backgrounds 3 `border-image-repeat: repeat` centres the
        // tiles in the area, so a box narrower than the image shows its
        // middle.
        Axis::Tile if out >= src => (0..src).collect(),
        Axis::Tile => (0..out)
            .map(|x| u32::try_from((u64::from(x) * 2 + 1 + u64::from(src - out)) / 2).unwrap_or(0))
            .collect(),
        Axis::Slice(a, b) => {
            let mid_src = src - a - b;
            let mut v = Vec::new();
            let (a_out, b_out) = if out >= a + b {
                (a, b)
            } else {
                let a_out =
                    u32::try_from(u64::from(out) * u64::from(a) / u64::from(a + b)).unwrap_or(0);
                (a_out, out - a_out)
            };
            v.extend((0..a_out).map(|x| nearest(x, a_out, a)));
            v.extend((0..b_out).map(|x| src - b + nearest(x, b_out, b)));
            let mid_out = out - a_out - b_out;
            v.extend((0..mid_out).map(|x| a + nearest(x, mid_out, mid_src)));
            v
        }
    };
    v.sort_unstable();
    v.dedup();
    v
}

/// The axes of `fit`, after checking nine-slice insets leave a middle.
fn axes(fit: RegionFit, width: u32, height: u32) -> Option<(Axis, Axis)> {
    match fit {
        RegionFit::Stretch => Some((Axis::Stretch, Axis::Stretch)),
        RegionFit::Tile => Some((Axis::Tile, Axis::Tile)),
        RegionFit::NineSlice(Insets {
            top,
            right,
            bottom,
            left,
        }) => {
            let (l, r) = (u32::from(left), u32::from(right));
            let (t, b) = (u32::from(top), u32::from(bottom));
            (l + r < width && t + b < height).then_some((Axis::Slice(l, r), Axis::Slice(t, b)))
        }
    }
}

/// A resolved variant's colours and dimensions, by registry id.
struct Tokens<'a> {
    colours: BTreeMap<&'a str, [u8; 4]>,
    dims: BTreeMap<&'a str, u32>,
}

impl<'a> Tokens<'a> {
    fn new(resolved: &'a [(&'a TokenEntry, TokenValue)]) -> Self {
        let mut colours = BTreeMap::new();
        let mut dims = BTreeMap::new();
        for (e, v) in resolved {
            match v {
                TokenValue::Color { r, g, b, a } => {
                    colours.insert(e.id, [*r, *g, *b, *a]);
                }
                TokenValue::Dimension(px) => {
                    dims.insert(e.id, u32::try_from(*px).unwrap_or(0));
                }
                _ => {}
            }
        }
        Self { colours, dims }
    }

    fn side(&self, extent: Extent, window: u32) -> u32 {
        match extent {
            Extent::Window => window,
            Extent::Token(id) => self.dims.get(id).copied().unwrap_or(0),
        }
        .max(1)
    }
}

/// The lowest contrast of `text` over `backing`, `tint` and every sampled
/// pixel of `image`, each over a black and a white desktop.
fn worst_contrast(
    image: &RgbaImage,
    cols: &[u32],
    rows: &[u32],
    tint: Option<[u8; 4]>,
    backing: [u8; 4],
    text: [u8; 4],
) -> f64 {
    let mut worst = f64::INFINITY;
    let mut seen = std::collections::HashSet::new();
    for y in rows {
        for x in cols {
            let p = image.get_pixel_checked(*x, *y).map_or([0; 4], |p| p.0);
            if !seen.insert(p) {
                continue;
            }
            for desktop in [[0u8; 3], [255; 3]] {
                let mut under = colour::over(p, desktop);
                if let Some(t) = tint {
                    under = colour::over(t, under);
                }
                let ground = colour::over(backing, under);
                worst = worst.min(colour::contrast(colour::over(text, ground), ground));
            }
        }
    }
    worst
}

/// Every frame of a canonical image.
fn frames(name: &str, bytes: &[u8]) -> Result<Vec<RgbaImage>, SkinError> {
    let kind = match extension(name).as_deref() {
        Some("gif") => ImageKind::Gif,
        _ => ImageKind::Png,
    };
    let limits = DecodeLimits {
        max_width: MAX_IMAGE_SIDE,
        max_height: MAX_IMAGE_SIDE,
        max_pixels: MAX_IMAGE_PIXELS,
        max_alloc: u64::from(MAX_IMAGE_FRAMES) * MAX_IMAGE_PIXELS * 4,
        frames: Frames::AtMost(MAX_IMAGE_FRAMES),
    };
    let decoded = decode_limited(bytes, kind, &limits).map_err(|e| {
        SkinError::new(
            name,
            SkinRule::ImageDecode {
                detail: e.to_string(),
            },
        )
    })?;
    Ok(match decoded {
        Decoded::Still(image) => vec![image.into_rgba8()],
        Decoded::Animated { first, rest } => std::iter::once(first)
            .chain(rest)
            .map(rekindle_image::Frame::into_buffer)
            .collect(),
    })
}

/// The colour token `id` names, or a refusal of the manifest field.
fn colour_token(tokens: &Tokens<'_>, field: &str, id: &str) -> Result<[u8; 4], SkinError> {
    tokens.colours.get(id).copied().ok_or_else(|| {
        SkinError::new(
            MANIFEST,
            SkinRule::BadValue {
                field: field.to_owned(),
                expected: "a colour token of the registry".to_owned(),
            },
        )
    })
}

/// Text over bitmaps (F83) for every region of `manifest` on a resolved
/// variant. `images` are the canonical images of [`crate::images::validate`].
///
/// # Errors
/// An unknown region, a text region without `textBacking`, a backing or
/// tint that is not a colour token, nine-slice insets that leave no
/// middle, or the first text role below its floor at the worst pixel.
pub fn validate(
    manifest: &SkinManifest,
    images: &BTreeMap<String, Vec<u8>>,
    resolved: &[(&TokenEntry, TokenValue)],
) -> Result<(), SkinError> {
    let tokens = Tokens::new(resolved);
    for (id, region) in &manifest.regions {
        let slot = slot_of(id).ok_or_else(|| {
            SkinError::new(MANIFEST, SkinRule::UnknownRegion { region: id.clone() })
        })?;
        let tint = match &region.tint {
            Some(t) => Some(colour_token(&tokens, &format!("regions.{id}.tint"), t)?),
            None => None,
        };
        let backing_id = match (&region.text_backing, slot.text.is_empty()) {
            (Some(b), _) => Some(b.as_str()),
            (None, true) => None,
            (None, false) => {
                return Err(SkinError::new(
                    MANIFEST,
                    SkinRule::MissingTextBacking { region: id.clone() },
                ))
            }
        };
        let backing = match backing_id {
            Some(b) => Some(colour_token(
                &tokens,
                &format!("regions.{id}.textBacking"),
                b,
            )?),
            None => None,
        };
        state_sizes(id, slot, region, images)?;
        let paths = std::iter::once(&region.image).chain(region.states.values());
        for path in paths {
            let Some(bytes) = images.get(path) else {
                return Err(SkinError::new(
                    path.as_str(),
                    SkinRule::MissingEntry {
                        field: format!("regions.{id}"),
                    },
                ));
            };
            for frame in frames(path, bytes)? {
                let (w, h) = frame.dimensions();
                let Some((x_axis, y_axis)) = axes(region.fit, w, h) else {
                    let RegionFit::NineSlice(i) = region.fit else {
                        continue;
                    };
                    return Err(SkinError::new(
                        path.as_str(),
                        SkinRule::NineSliceInsets {
                            region: id.clone(),
                            top: i.top,
                            right: i.right,
                            bottom: i.bottom,
                            left: i.left,
                            width: w,
                            height: h,
                        },
                    ));
                };
                let Some(backing) = backing else { continue };
                let ctx = Measure {
                    region: id,
                    slot,
                    tokens: &tokens,
                    image: &frame,
                    axes: (x_axis, y_axis),
                    tint,
                    backing: (backing_id.unwrap_or_default(), backing),
                };
                ctx.check()
                    .map_err(|rule| SkinError::new(path.as_str(), rule))?;
            }
        }
    }
    Ok(())
}

/// Every state image of `region` is one `slot` has and is the size of the
/// normal image (so the region's nine-slice fits it the same way).
fn state_sizes(
    id: &str,
    slot: &RegionSlot,
    region: &RegionSpec,
    images: &BTreeMap<String, Vec<u8>>,
) -> Result<(), SkinError> {
    let size = |path: &String| -> Result<Option<(u32, u32)>, SkinError> {
        match images.get(path) {
            Some(bytes) => Ok(frames(path, bytes)?.first().map(RgbaImage::dimensions)),
            None => Ok(None),
        }
    };
    let normal = size(&region.image)?;
    for (state, path) in &region.states {
        if !slot.has(*state) {
            return Err(SkinError::new(
                MANIFEST,
                SkinRule::UnsupportedRegionState {
                    region: id.to_owned(),
                    state: state.key().to_owned(),
                },
            ));
        }
        if let (Some((nw, nh)), Some((w, h))) = (normal, size(path)?) {
            if (w, h) != (nw, nh) {
                return Err(SkinError::new(
                    path.as_str(),
                    SkinRule::RegionStateSize {
                        region: id.to_owned(),
                        state: state.key().to_owned(),
                        width: w,
                        height: h,
                        normal_width: nw,
                        normal_height: nh,
                    },
                ));
            }
        }
    }
    Ok(())
}

/// One region frame's measurement.
struct Measure<'a> {
    region: &'a str,
    slot: &'a RegionSlot,
    tokens: &'a Tokens<'a>,
    image: &'a RgbaImage,
    axes: (Axis, Axis),
    tint: Option<[u8; 4]>,
    backing: (&'a str, [u8; 4]),
}

impl Measure<'_> {
    fn check(&self) -> Result<(), SkinRule> {
        let (w, h) = self.image.dimensions();
        for win in &WINDOWS {
            for (side_w, side_h) in [win.min, win.max] {
                let bw = self.tokens.side(self.slot.width, side_w);
                let bh = self.tokens.side(self.slot.height, side_h);
                let cols = sampled(self.axes.0, bw, w);
                let rows = sampled(self.axes.1, bh, h);
                for t in self.slot.text {
                    let text = self.tokens.colours.get(t.token).copied().unwrap_or([0; 4]);
                    let ratio =
                        worst_contrast(self.image, &cols, &rows, self.tint, self.backing.1, text);
                    let required: u16 = if t.large { 300 } else { 450 };
                    if ratio * 100.0 < f64::from(required) {
                        return Err(SkinRule::TextOverBitmap {
                            region: self.region.to_owned(),
                            token: t.token.to_owned(),
                            backing: self.backing.0.to_owned(),
                            window: win.kind.to_owned(),
                            width: bw,
                            height: bh,
                            measured_x100: scaled(ratio, 100.0),
                            required_x100: u32::from(required),
                        });
                    }
                }
            }
        }
        Ok(())
    }
}

/// Every slot's text token is a colour of the registry.
#[must_use]
pub fn slot_tokens_are_colours(table: &[TokenEntry]) -> bool {
    SLOTS.iter().flat_map(|s| s.text).all(|t| {
        table
            .iter()
            .any(|e| e.id == t.token && matches!(e.ty, TokenType::Color))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ENTRIES;

    #[test]
    fn slots_name_registry_colours() {
        assert!(slot_tokens_are_colours(ENTRIES));
    }

    #[test]
    fn axes_sample_as_drawn() {
        assert_eq!(sampled(Axis::Stretch, 2, 4), vec![1, 3]);
        assert_eq!(sampled(Axis::Stretch, 8, 4), vec![0, 1, 2, 3]);
        // Centred tiles: a 3 px box over 10 px shows the middle.
        assert_eq!(sampled(Axis::Tile, 3, 10), vec![4, 5, 6]);
        assert_eq!(sampled(Axis::Tile, 12, 10).len(), 10);
        // Insets 2 + 2 of 10, a 20 px box: every pixel.
        assert_eq!(sampled(Axis::Slice(2, 2), 20, 10).len(), 10);
        // A 2 px box: the insets shrink to 1 + 1, the middle is not drawn.
        assert_eq!(sampled(Axis::Slice(2, 2), 2, 10), vec![1, 9]);
    }
}
