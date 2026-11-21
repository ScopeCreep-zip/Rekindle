//! Plan SK2 lead decision 15: a fixed tile's box and all its state images
//! are clipped to its parent, as Xfire clipped a tile to its window (XGlow's
//! `Gripper` `X=Bottom.RIGHT` `X=-20`, `Xglow.ini:229-233`: 34×30 art whose
//! right 14 and bottom 10 px lie past the window, which the original never
//! shows). The art is cropped here, so the served images are the visible
//! part; a clipped node stays in the tree with its role, so a resize part
//! keeps taking input over what of it is inside (its hit area,
//! [`super::validate`], is grown from that box). A clipped part then lies
//! inside its parent, so the overhang ([`super::fit`]) no longer counts it.
//! A tile stretched on either axis is not clipped: its edges place it past
//! its parent on purpose (lead decision 14, Total Chaos's rows).

use std::collections::BTreeMap;

use rekindle_image::RgbaImage;
use rekindle_types::dto::skin::{SkinWindow, TileAxis};
use rekindle_types::dto::skin_tree::{Anchor, NodeArt, NodeSpan, SkinNode};

use crate::import::ImportError;
use crate::shape;

/// The images the tree's art was encoded from, by package path.
pub(super) type Images = BTreeMap<String, RgbaImage>;

/// What the tree's art is cropped from and re-encoded into.
pub(super) struct Art<'a> {
    pub window: SkinWindow,
    pub images: &'a Images,
    pub files: &'a mut BTreeMap<String, Vec<u8>>,
}

/// How much of a fixed `span` lies before its parent's start and past its
/// end (a parent `len` px long, or one that stretches with the window,
/// `None`, where only the offset can lay it outside), at most its size.
fn cut(span: NodeSpan, len: Option<u16>) -> (u16, u16) {
    let NodeSpan::Fixed { at, size } = span else {
        return (0, 0);
    };
    let (before, after) = match (len, at) {
        (Some(len), _) => {
            let (s, l) = span.place(i32::from(len));
            ((-s).max(0), (s + l - i32::from(len)).max(0))
        }
        (None, Anchor::Start(o)) => ((-i32::from(o)).max(0), 0),
        (None, Anchor::End(o)) => (0, (-i32::from(o)).max(0)),
        (None, Anchor::Centre(_)) => (0, 0),
    };
    let b = u16::try_from(before).unwrap_or(u16::MAX).min(size);
    let a = u16::try_from(after).unwrap_or(u16::MAX).min(size - b);
    (b, a)
}

/// `span` less `cut` (in a parent `len` px long, when fixed).
fn clipped(span: NodeSpan, (b, a): (u16, u16), len: Option<u16>) -> NodeSpan {
    let NodeSpan::Fixed { at, size } = span else {
        return span;
    };
    let size_left = size - b - a;
    let at = match (at, len) {
        (Anchor::Start(o), _) => Anchor::Start(o.saturating_add_unsigned(b)),
        (Anchor::End(o), _) => Anchor::End(o.saturating_add_unsigned(a)),
        (Anchor::Centre(_), Some(len)) => {
            let start = span.place(i32::from(len)).0 + i32::from(b);
            Anchor::Start(i16::try_from(start).unwrap_or(i16::MAX))
        }
        (Anchor::Centre(o), None) => Anchor::Centre(o),
    };
    NodeSpan::Fixed {
        at,
        size: size_left,
    }
}

/// A child's `span` in its parent's frame after the parent lost `cut` and
/// is now `len` px long.
fn shifted(span: NodeSpan, (b, a): (u16, u16), len: u16) -> NodeSpan {
    let (b16, a16) = (
        i16::try_from(b).unwrap_or(i16::MAX),
        i16::try_from(a).unwrap_or(i16::MAX),
    );
    match span {
        NodeSpan::Fixed {
            at: Anchor::Start(o),
            size,
        } => NodeSpan::Fixed {
            at: Anchor::Start(o.saturating_sub(b16)),
            size,
        },
        NodeSpan::Fixed {
            at: Anchor::End(o),
            size,
        } => NodeSpan::Fixed {
            at: Anchor::End(o.saturating_sub(a16)),
            size,
        },
        NodeSpan::Fixed {
            at: Anchor::Centre(_),
            size,
        } => {
            let old = len + b + a;
            let start = span.place(i32::from(old)).0 - i32::from(b);
            NodeSpan::Fixed {
                at: Anchor::Start(i16::try_from(start).unwrap_or(i16::MAX)),
                size,
            }
        }
        NodeSpan::Stretch { start, end } => NodeSpan::Stretch {
            start: start.saturating_sub(b16),
            end: end.saturating_sub(a16),
        },
    }
}

/// The package image `path` cropped to `[x0, x0 + w) × [y0, y0 + h)` and
/// re-encoded at the same path; `None` when nothing of it is left to show.
fn crop(
    art: &mut Art<'_>,
    path: &str,
    xy: (u16, u16),
    wh: (u16, u16),
) -> Result<Option<String>, ImportError> {
    let Some(img) = art.images.get(path) else {
        return Ok(Some(path.to_owned()));
    };
    let part = rekindle_image::DynamicImage::ImageRgba8(img.clone())
        .crop_imm(
            u32::from(xy.0),
            u32::from(xy.1),
            u32::from(wh.0),
            u32::from(wh.1),
        )
        .to_rgba8();
    let piece = path
        .rsplit_once('/')
        .map_or(path, |(_, f)| f)
        .trim_end_matches(".png");
    let axes = (TileAxis::Start { offset: 0 }, TileAxis::Start { offset: 0 });
    art.files.remove(path);
    Ok(
        shape::tile(art.window, piece, &part, axes, true)?.map(|(t, png)| {
            art.files.insert(t.image.clone(), png);
            t.image
        }),
    )
}

/// `node`'s art cropped by `(x cut, y cut)`; `None` when its normal image
/// has nothing left to show.
fn crop_art(
    art: &mut Art<'_>,
    a: NodeArt,
    cuts: [(u16, u16); 2],
    wh: (u16, u16),
) -> Result<Option<NodeArt>, ImportError> {
    let at = (cuts[0].0, cuts[1].0);
    let Some(image) = crop(art, &a.image, at, wh)? else {
        for p in a.states.values() {
            art.files.remove(p);
        }
        return Ok(None);
    };
    let mut states = BTreeMap::new();
    for (k, p) in a.states {
        if let Some(p) = crop(art, &p, at, wh)? {
            states.insert(k, p);
        }
    }
    Ok(Some(NodeArt { image, states }))
}

/// Clips each of `nodes` (in a parent `(w, h)` px, or `None` on an axis
/// that stretches with the window) to its parent, then its children to it.
///
/// # Errors
/// A cropped image that does not encode.
pub(super) fn clip(
    nodes: &mut [SkinNode],
    parent: (Option<u16>, Option<u16>),
    art: &mut Art<'_>,
) -> Result<(), ImportError> {
    for n in nodes {
        // A tile stretched on either axis is placed by its edges, not its
        // art (Total Chaos's `BottomBar` `ResizeX="100"`, 5 px past the
        // box on purpose: lead decision 14), so only a wholly fixed one is.
        let fixed = matches!((n.x, n.y), (NodeSpan::Fixed { .. }, NodeSpan::Fixed { .. }));
        let cuts = if fixed {
            [cut(n.x, parent.0), cut(n.y, parent.1)]
        } else {
            [(0, 0); 2]
        };
        n.x = clipped(n.x, cuts[0], parent.0);
        n.y = clipped(n.y, cuts[1], parent.1);
        let len = |s: NodeSpan| match s {
            NodeSpan::Fixed { size, .. } => Some(size),
            NodeSpan::Stretch { .. } => None,
        };
        let own = (len(n.x), len(n.y));
        if cuts != [(0, 0); 2] {
            if let ((Some(w), Some(h)), Some(a)) = (own, n.art.take()) {
                n.art = crop_art(art, a, cuts, (w, h))?;
            }
            for c in &mut n.children {
                if let Some(w) = own.0 {
                    c.x = shifted(c.x, cuts[0], w);
                }
                if let Some(h) = own.1 {
                    c.y = shifted(c.y, cuts[1], h);
                }
            }
        }
        clip(&mut n.children, own, art)?;
    }
    Ok(())
}
