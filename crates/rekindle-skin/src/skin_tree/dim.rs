//! The Disabled look the serve draws for an interactive node whose pack
//! draws none (plan E7.7 CR1 / DIS3, owner rule: a disabled button is
//! visibly disabled with the Host's reason, never hidden): a dimmed copy of
//! what is painted at the node's box, made at serve time and never written
//! into a package ([`super::disabled`]).
//!
//! The shell's `opacity: 0.5` blends a node with what lies beneath it, so
//! it changes nothing where the node's art equals the art under it: Alloy's
//! info-popup `HostChatButton` and Combat's `JoinButton@Friends` are opaque
//! copies of their parent tab's art at the same box (0 px change, research
//! `research-dis3-transparent-art-2026-11-26.md`). The copy is drawn as the
//! node's own Disabled layer instead, so it replaces what is seen there:
//! - a node with Normal art of its own: each of its pieces dimmed, in its
//!   slot, so the copy lays exactly as the Normal set does;
//! - an art-less node (Alloy's `JoinChatButton`, whose tab alone draws
//!   what is seen): its nearest ancestor's Normal set laid at that
//!   ancestor's box ([`pieces::draw`], as the stylesheet lays it) in the
//!   window at its served minimum, cropped to the node's box, dimmed.
//!
//! The dim is one of two standard filters, per node the one that changes
//! more pixels of its copy visibly ([`changed_pixels`]): `brightness(0.5)`
//! moves every channel above 64 by more than the channel delta (32,
//! `disabled::CHANNEL_DELTA`) (Alloy's mid-grey plate: 1487 of 1575 px), `contrast(0.5)` about
//! mid-grey every channel below 64 or above 192 (dark Combat: 1394 of
//! 1850 px, where `brightness(0.5)` reaches 442). Both are monotone, so an
//! icon keeps its light and dark order; grayscale is never one, since the
//! grey packs are already neutral (0 px). No single continuous per-channel
//! map moves every value by more than the delta (each has a fixed point),
//! so the pair covers both ends of the range. The copy is served only when
//! it clears [`VISIBLE_PIXELS`], proven here with [`changed_pixels`].

use std::collections::btree_map::Entry;
use std::collections::BTreeMap;

use rekindle_image::{encode_png, DynamicImage, RgbaImage};
use rekindle_types::dto::skin::{SkinError, SkinRule};
use rekindle_types::dto::skin_tree::{NodePiece, NodePieces, PieceSlot, SkinNode};

use super::disabled::{changed_pixels, load, Measured, VISIBLE_PIXELS};
use super::pieces;
use crate::package::Package;

/// The images a serve made, by name: each a canonical RGBA8 PNG. Served
/// beside the package's own images, at the address of its bytes
/// ([`crate::serve::body`]).
pub type Made = BTreeMap<String, Vec<u8>>;

/// The name prefix of a made image: never a package path (those name the
/// package's own files), so a made image never shadows one.
pub(super) const MADE: &str = "served:disabled/";

/// A dimming filter, as CSS defines it on each colour channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dim {
    /// `brightness(0.5)`.
    Darken,
    /// `contrast(0.5)`: halfway to mid-grey.
    Flatten,
}

impl Dim {
    const ALL: [Self; 2] = [Self::Darken, Self::Flatten];

    const fn channel(self, v: u8) -> u8 {
        match self {
            Self::Darken => v / 2,
            Self::Flatten => v / 2 + 64,
        }
    }

    /// `src` with each colour channel dimmed; alpha kept.
    fn image(self, src: &RgbaImage) -> RgbaImage {
        let mut out = src.clone();
        for p in out.pixels_mut() {
            for c in &mut p.0[..3] {
                *c = self.channel(*c);
            }
        }
        out
    }
}

/// What is painted under an art-less node: its nearest ancestor's Normal
/// set and that ancestor's box in the window.
#[derive(Debug, Clone)]
pub struct Under {
    pub set: NodePieces,
    pub at: [i32; 4],
}

/// The Disabled set the serve draws for `n`, laid at `at` in the window,
/// `under` what its nearest ancestor with art paints, and its made images
/// added to `measured`; `None` when nothing is painted at its box or
/// neither dim changes [`VISIBLE_PIXELS`] of it.
///
/// # Errors
/// A Normal piece missing from `package` or not decoding, or a copy not
/// encoding.
pub fn copy(
    n: &SkinNode,
    at: [i32; 4],
    under: Option<&Under>,
    package: &Package<'_>,
    measured: &mut Measured,
) -> Result<Option<NodePieces>, SkinError> {
    let normal = match n.art.as_ref().filter(|a| !a.normal.is_empty()) {
        Some(art) => {
            for p in art.normal.values() {
                load(p, package, &mut measured.frames)?;
            }
            art.normal.clone()
        }
        None => match under {
            Some(u) => match crop(u, at, package, measured)? {
                Some(set) => set,
                None => return Ok(None),
            },
            None => return Ok(None),
        },
    };
    let mut best: Option<(u64, NodePieces)> = None;
    for dim in Dim::ALL {
        let set = dimmed(&normal, dim, measured);
        let changed = changed_pixels(&normal, &set, package, measured)?;
        if best.as_ref().is_none_or(|(b, _)| changed > *b) {
            best = Some((changed, set));
        }
    }
    let Some((_, set)) = best.filter(|(changed, _)| *changed >= VISIBLE_PIXELS) else {
        return Ok(None);
    };
    for p in set.values() {
        if let Entry::Vacant(slot) = measured.made.entry(p.image.clone()) {
            let png = encode_png(&DynamicImage::ImageRgba8(measured.frames[&p.image].clone()))
                .map_err(|e| {
                    SkinError::new(
                        p.image.as_str(),
                        SkinRule::ImageDecode {
                            detail: e.to_string(),
                        },
                    )
                })?;
            slot.insert(png);
        }
    }
    Ok(Some(set))
}

/// The name of a made image: its pixels' content address.
fn name(image: &RgbaImage) -> String {
    let mut h = blake3::Hasher::new();
    h.update(&image.width().to_le_bytes());
    h.update(&image.height().to_le_bytes());
    h.update(image.as_raw());
    format!("{MADE}{}.png", h.finalize().to_hex())
}

/// Each piece of `set` dimmed by `dim`, in its slot at its size, its
/// image decoded into `measured` under its made name.
fn dimmed(set: &NodePieces, dim: Dim, measured: &mut Measured) -> NodePieces {
    set.iter()
        .map(|(slot, p)| {
            let image = dim.image(&measured.frames[&p.image]);
            let named = name(&image);
            measured.frames.entry(named.clone()).or_insert(image);
            (
                *slot,
                NodePiece {
                    image: named,
                    width: p.width,
                    height: p.height,
                },
            )
        })
        .collect()
}

/// `under`'s set laid at its box and cropped to the box `at`, as one
/// middle piece the node's size; `None` for an empty box.
fn crop(
    under: &Under,
    at: [i32; 4],
    package: &Package<'_>,
    measured: &mut Measured,
) -> Result<Option<NodePieces>, SkinError> {
    let (Ok(w), Ok(h)) = (u16::try_from(at[2]), u16::try_from(at[3])) else {
        return Ok(None);
    };
    if w == 0 || h == 0 {
        return Ok(None);
    }
    for p in under.set.values() {
        load(p, package, &mut measured.frames)?;
    }
    let mut out = RgbaImage::new(u32::from(w), u32::from(h));
    let (dx, dy) = (at[0] - under.at[0], at[1] - under.at[1]);
    for c in pieces::draw(&under.set, under.at[2], under.at[3]) {
        let Some(piece) = under.set.get(&c.slot) else {
            continue;
        };
        let src = &measured.frames[&piece.image];
        for y in 0..c.at[3] {
            for x in 0..c.at[2] {
                let (Ok(ox), Ok(oy)) = (
                    u32::try_from(c.at[0] + x - dx),
                    u32::try_from(c.at[1] + y - dy),
                ) else {
                    continue;
                };
                let (Ok(sx), Ok(sy)) = (u32::try_from(c.src.0 + x), u32::try_from(c.src.1 + y))
                else {
                    continue;
                };
                if ox >= out.width() || oy >= out.height() {
                    continue;
                }
                if let Some(px) = src.get_pixel_checked(sx, sy) {
                    out.put_pixel(ox, oy, *px);
                }
            }
        }
    }
    let named = name(&out);
    measured.frames.entry(named.clone()).or_insert(out);
    Ok(Some(NodePieces::from([(
        PieceSlot::Middle,
        NodePiece {
            image: named,
            width: w,
            height: h,
        },
    )])))
}

#[cfg(test)]
#[path = "dim_tests.rs"]
mod tests;
