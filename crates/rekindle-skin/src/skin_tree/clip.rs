//! Plan SK2 lead decision 15: a fixed tile's box and all its state images
//! are clipped to its parent, as Xfire clipped a tile to its window (XGlow's
//! `Gripper` `X=Bottom.RIGHT` `X=-20`, `Xglow.ini:229-233`: 34×30 art whose
//! right 14 and bottom 10 px lie past the window, which the original never
//! shows). The art is cropped here, so the served images are the visible
//! part; a clipped node stays in the tree with its role, so a resize part
//! keeps taking input over what of it is inside (its hit area,
//! [`super::validate`], is grown from that box). A clipped part then lies
//! inside its parent, so the overhang ([`super::fit`]) no longer counts it.
//! A tile stretched on one axis is still clipped on its fixed axis (lead
//! decision 27, correcting decision 14's "a stretched tile is not clipped"
//! against the original's pixels): Total Chaos's `BottomBar` `ResizeX="100"`
//! is 66 px tall under `Content` `IndentBottom="61"`, and the original shows
//! its top 61 rows at the window's bottom edge, never growing the window
//! (`skinref/total-chaos/buddy-list-1.png`). Its stretched axis is placed by
//! its edges (the rows' `OutdentLeft="1"`) and is never cut; its one-row
//! piece set is trimmed row by row, each piece kept in its slot.

use std::collections::BTreeMap;

use rekindle_image::RgbaImage;
use rekindle_types::dto::skin::{SkinWindow, TileAxis};
use rekindle_types::dto::skin_tree::{
    Anchor, NodeArt, NodePiece, NodePieces, NodeSpan, PieceSlot, SkinNode,
};

use super::import::bad;
use super::pieces;

use crate::import::ImportError;
use crate::shape;

/// The images the tree's art was encoded from, by package path.
pub(super) type Images = BTreeMap<String, RgbaImage>;

/// What the tree's art is cropped from and re-encoded into.
pub(super) struct Art<'a> {
    pub window: SkinWindow,
    pub images: &'a mut Images,
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

/// The set `p` drawn in a `wh` box ([`super::pieces::draw`], the one
/// geometry the stylesheet and the outline read); `None` when one of its
/// images is not among those the tree encoded.
fn composed(images: &Images, p: &NodePieces, (w, h): (u16, u16)) -> Option<RgbaImage> {
    let mut canvas = RgbaImage::new(u32::from(w), u32::from(h));
    for c in pieces::draw(p, i32::from(w), i32::from(h)) {
        let img = images.get(&p.get(&c.slot)?.image)?;
        let (sx, sy) = (u32::try_from(c.src.0).ok()?, u32::try_from(c.src.1).ok()?);
        let (ax, ay) = (u32::try_from(c.at[0]).ok()?, u32::try_from(c.at[1]).ok()?);
        for dy in 0..u32::try_from(c.at[3]).ok()? {
            for dx in 0..u32::try_from(c.at[2]).ok()? {
                canvas.put_pixel(ax + dx, ay + dy, *img.get_pixel(sx + dx, sy + dy));
            }
        }
    }
    Some(canvas)
}

/// The set `p`, drawn at its fixed box `full`, cropped to `[x0, x0 + w) ×
/// [y0, y0 + h)` and re-encoded as one `middle` piece the cropped box's size
/// (a fixed box draws it exactly once); `None` when nothing of it is left
/// to show.
fn crop(
    art: &mut Art<'_>,
    p: NodePieces,
    full: (u16, u16),
    xy: (u16, u16),
    wh: (u16, u16),
) -> Result<Option<NodePieces>, ImportError> {
    let Some(name) = p
        .get(&PieceSlot::Middle)
        .or_else(|| p.values().next())
        .map(|x| x.image.clone())
    else {
        return Ok(None);
    };
    let Some(img) = composed(art.images, &p, full) else {
        return Ok(Some(p));
    };
    for x in p.values() {
        art.files.remove(&x.image);
    }
    let part = rekindle_image::DynamicImage::ImageRgba8(img)
        .crop_imm(
            u32::from(xy.0),
            u32::from(xy.1),
            u32::from(wh.0),
            u32::from(wh.1),
        )
        .to_rgba8();
    let piece = name
        .rsplit_once('/')
        .map_or(name.as_str(), |(_, f)| f)
        .trim_end_matches(".png");
    let axes = (TileAxis::Start { offset: 0 }, TileAxis::Start { offset: 0 });
    Ok(
        shape::tile(art.window, piece, &part, axes, true)?.map(|(t, png)| {
            art.files.insert(t.image.clone(), png);
            NodePieces::from([(
                PieceSlot::Middle,
                NodePiece {
                    image: t.image,
                    width: wh.0,
                    height: wh.1,
                },
            )])
        }),
    )
}

/// `node`'s art cropped by `(x cut, y cut)` to `wh`; `None` when its
/// normal set has nothing left to show.
fn crop_art(
    art: &mut Art<'_>,
    a: NodeArt,
    cuts: [(u16, u16); 2],
    wh: (u16, u16),
) -> Result<Option<NodeArt>, ImportError> {
    let at = (cuts[0].0, cuts[1].0);
    let full = (wh.0 + cuts[0].0 + cuts[0].1, wh.1 + cuts[1].0 + cuts[1].1);
    let Some(normal) = crop(art, a.normal, full, at, wh)? else {
        for p in a.states.values().flat_map(|s| s.values()) {
            art.files.remove(&p.image);
        }
        return Ok(None);
    };
    let mut states = BTreeMap::new();
    for (k, p) in a.states {
        if let Some(p) = crop(art, p, full, at, wh)? {
            states.insert(k, p);
        }
    }
    Ok(Some(NodeArt { normal, states }))
}

/// The slots a set may hold to be trimmed across `y` (rows, a one-row set:
/// `left`, `middle`, `right`) or down (columns, a one-column set).
fn one_band(p: &NodePieces, y: bool) -> bool {
    use PieceSlot as S;
    let ok: [S; 3] = if y {
        [S::Left, S::Middle, S::Right]
    } else {
        [S::Top, S::Middle, S::Bottom]
    };
    p.keys().all(|s| ok.contains(s))
}

/// The set `p` of a box stretched on the other axis, trimmed on its fixed
/// axis (`y` rows, else columns) by `(before, _)` to `len` px: each piece
/// keeps its slot and is re-encoded as the `len` px it shows. In a one-row
/// set every piece is laid from the box's top and repeated down
/// ([`super::pieces::cell`]), so the box's row `r` shows the piece's row
/// `r mod height`, and the trimmed piece's row `r` is the source's
/// `(before + r) mod height`; likewise for a one-column set's columns.
fn trim(
    art: &mut Art<'_>,
    p: NodePieces,
    y: bool,
    before: u16,
    len: u16,
) -> Result<NodePieces, ImportError> {
    if !one_band(&p, y) {
        return Err(bad(format!(
            "a tile stretched on one axis and clipped on the other has a piece set \
             that is not one {} ({:?})",
            if y { "row" } else { "column" },
            p.keys().collect::<Vec<_>>()
        )));
    }
    let mut out = NodePieces::new();
    for (slot, x) in p {
        let src = art
            .images
            .get(&x.image)
            .ok_or_else(|| bad(format!("{} is not among the tree's images", x.image)))?;
        let (pw, ph) = (src.width(), src.height());
        let (w, h) = if y {
            (pw, u32::from(len))
        } else {
            (u32::from(len), ph)
        };
        let img = RgbaImage::from_fn(w, h, |c, r| {
            if y {
                *src.get_pixel(c, (u32::from(before) + r) % ph)
            } else {
                *src.get_pixel((u32::from(before) + c) % pw, r)
            }
        });
        art.files.remove(&x.image);
        let piece = x
            .image
            .rsplit_once('/')
            .map_or(x.image.as_str(), |(_, f)| f)
            .trim_end_matches(".png")
            .to_owned();
        let axes = (TileAxis::Start { offset: 0 }, TileAxis::Start { offset: 0 });
        if let Some((t, png)) = shape::tile(art.window, &piece, &img, axes, true)? {
            art.files.insert(t.image.clone(), png);
            let (width, height) = if y { (x.width, len) } else { (len, x.height) };
            out.insert(
                slot,
                NodePiece {
                    image: t.image,
                    width,
                    height,
                },
            );
        }
    }
    Ok(out)
}

/// `node`'s art, stretched on one axis, trimmed on its fixed axis (`y`
/// rows, else columns) by `cut` to `len` px; `None` when its normal set has
/// nothing left to show.
fn trim_art(
    art: &mut Art<'_>,
    a: NodeArt,
    y: bool,
    cut: (u16, u16),
    len: u16,
) -> Result<Option<NodeArt>, ImportError> {
    let normal = trim(art, a.normal, y, cut.0, len)?;
    let mut states = BTreeMap::new();
    for (k, p) in a.states {
        let p = trim(art, p, y, cut.0, len)?;
        if !p.is_empty() {
            states.insert(k, p);
        }
    }
    if normal.is_empty() {
        for p in states.values().flat_map(|s| s.values()) {
            art.files.remove(&p.image);
        }
        return Ok(None);
    }
    Ok(Some(NodeArt { normal, states }))
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
        // Each fixed axis is clipped; a stretched axis is placed by its
        // edges and never cut ([`cut`]), lead decision 27.
        // A 0 px parent is sized by its text or children at render (an
        // art-less menu bar), so it bounds nothing here.
        let parent = (parent.0.filter(|l| *l > 0), parent.1.filter(|l| *l > 0));
        // A tile stretched on one axis is clipped on its fixed one for its
        // art (decision 27); an art-less one is placed by its text
        // (xSLIMFIRE's `FileMenu` `ResizeY="100"`, its 12 px text in the
        // 2 px `XfireMenuText`, `xSLIMFIREmainwindow.xml:48-50`, whose
        // menu the original shows), so only a wholly fixed one is.
        let fixed = matches!((n.x, n.y), (NodeSpan::Fixed { .. }, NodeSpan::Fixed { .. }));
        let cuts = if fixed || n.art.is_some() {
            [cut(n.x, parent.0), cut(n.y, parent.1)]
        } else {
            [(0, 0); 2]
        };
        if cuts[0] != (0, 0) {
            n.x = clipped(n.x, cuts[0], parent.0);
        }
        if cuts[1] != (0, 0) {
            n.y = clipped(n.y, cuts[1], parent.1);
        }
        let len = |s: NodeSpan| match s {
            NodeSpan::Fixed { size, .. } => Some(size),
            NodeSpan::Stretch { .. } => None,
        };
        let own = (len(n.x), len(n.y));
        if cuts != [(0, 0); 2] {
            if let Some(a) = n.art.take() {
                n.art = match own {
                    (Some(w), Some(h)) => crop_art(art, a, cuts, (w, h))?,
                    (None, Some(h)) => trim_art(art, a, true, cuts[1], h)?,
                    (Some(w), None) => trim_art(art, a, false, cuts[0], w)?,
                    // A stretched axis is never cut, so no cut leaves both.
                    (None, None) => Some(a),
                };
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
