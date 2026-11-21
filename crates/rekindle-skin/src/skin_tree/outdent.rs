//! Plan SK2 lead decision 29: an `Outdent*` never grows the window. Art an
//! outdent pushes past its parent is clipped on either axis, as Xfire
//! clipped every tile to its parent window (this finishes replacing
//! decision 14). Total Chaos's rows `X="0" OutdentLeft="1" ResizeX="100"`
//! (`Total_Chaos/mainwindow.xml:2,11,14,22,37`) start at x = −1; the
//! original is 470 px wide with their first column cut, its 8-px title
//! repeat at phase x ≡ 2 (mod 8), the end of the 75-px left cap laid from
//! −1 (spec A5; `skinref/total-chaos/buddy-list-1.png`). The stretched
//! box moves inside its parent, each start-edge piece loses the columns
//! (rows) that lay outside and each end-edge piece its last ones, so every
//! piece that stays shows the same pixels where it showed them before.
//! This pass runs on the window's own tiles, where an outdent would grow
//! the window; a nested tile's negative span (StealthX's `FileMenu`
//! `Y="-4"` in `XfireMenuText`, `popups.xml:5-6`) is left to
//! [`super::clip`], which clips fixed axes only.

use rekindle_types::dto::skin::TileAxis;
use rekindle_types::dto::skin_tree::{
    Anchor, NodeArt, NodePiece, NodePieces, NodeSpan, PieceSlot, SkinNode,
};

use super::clip::Art;
use crate::import::ImportError;
use crate::shape;

/// How far a stretched `span` lies before its parent's start and past its
/// end (its negative insets, the `Outdent*`s).
fn outdent(span: NodeSpan) -> (u16, u16) {
    let NodeSpan::Stretch { start, end } = span else {
        return (0, 0);
    };
    let past = |v: i16| u16::try_from((-i32::from(v)).max(0)).unwrap_or(u16::MAX);
    (past(start), past(end))
}

/// `span` moved inside its parent.
fn inside(span: NodeSpan) -> NodeSpan {
    match span {
        NodeSpan::Stretch { start, end } => NodeSpan::Stretch {
            start: start.max(0),
            end: end.max(0),
        },
        fixed @ NodeSpan::Fixed { .. } => fixed,
    }
}

/// A child's `span` in its parent's frame after the parent's stretched box
/// lost `(b, a)` px at its start and end. A centred child keeps its place
/// to the pixel when `a − b` is even; when odd, the half pixel rounds as the
/// anchors do (`anchor.rs`, `(start − end).div_euclid(2)`).
fn shift_child(span: NodeSpan, (b, a): (u16, u16)) -> NodeSpan {
    let i = |v: u16| i16::try_from(v).unwrap_or(i16::MAX);
    match span {
        NodeSpan::Fixed {
            at: Anchor::Start(o),
            size,
        } => NodeSpan::Fixed {
            at: Anchor::Start(o.saturating_sub(i(b))),
            size,
        },
        NodeSpan::Fixed {
            at: Anchor::End(o),
            size,
        } => NodeSpan::Fixed {
            at: Anchor::End(o.saturating_sub(i(a))),
            size,
        },
        NodeSpan::Fixed {
            at: Anchor::Centre(o),
            size,
        } => NodeSpan::Fixed {
            at: Anchor::Centre(o.saturating_add((i(a) - i(b)).div_euclid(2))),
            size,
        },
        NodeSpan::Stretch { start, end } => NodeSpan::Stretch {
            start: start.saturating_sub(i(b)),
            end: end.saturating_sub(i(a)),
        },
    }
}

/// Where `slot` lies across the axis (`x` columns, else rows): its start
/// edge (`Some(false)`), its end edge (`Some(true)`), or the repeated
/// middle (`None`).
fn edge(slot: PieceSlot, x: bool) -> Option<bool> {
    use PieceSlot as S;
    let (start, end) = if x {
        (
            matches!(slot, S::TopLeft | S::Left | S::BottomLeft),
            matches!(slot, S::TopRight | S::Right | S::BottomRight),
        )
    } else {
        (
            matches!(slot, S::TopLeft | S::Top | S::TopRight),
            matches!(slot, S::BottomLeft | S::Bottom | S::BottomRight),
        )
    };
    match (start, end) {
        (true, _) => Some(false),
        (_, true) => Some(true),
        _ => None,
    }
}

/// The set `p` of a box that lost `(b, a)` px on axis `x` (else y): each
/// start-edge piece drops its first `b` lines, each end-edge piece its last
/// `a`; with no start-edge piece the middle ones start `b` lines later in
/// their repeat. A piece with nothing left is dropped.
fn cut_set(
    art: &mut Art<'_>,
    set: NodePieces,
    x: bool,
    (b, a): (u16, u16),
) -> Result<NodePieces, ImportError> {
    let has_start = set.keys().any(|s| edge(*s, x) == Some(false));
    let mut out = NodePieces::new();
    for (slot, piece) in set {
        let Some(src) = art.images.get(&piece.image).cloned() else {
            return Err(super::import::bad(format!(
                "{} is not among the tree's images",
                piece.image
            )));
        };
        let len = if x { src.width() } else { src.height() };
        let (skip, keep, wrap) = match edge(slot, x) {
            Some(false) => (u32::from(b), len.saturating_sub(u32::from(b)), false),
            Some(true) => (0, len.saturating_sub(u32::from(a)), false),
            None if !has_start && b > 0 => (u32::from(b), len, true),
            None => {
                out.insert(slot, piece);
                continue;
            }
        };
        art.files.remove(&piece.image);
        if keep == 0 {
            continue;
        }
        let (cols, rows) = if x {
            (keep, src.height())
        } else {
            (src.width(), keep)
        };
        let img = rekindle_image::RgbaImage::from_fn(cols, rows, |c, r| {
            let at = |v: u32| if wrap { (skip + v) % len } else { skip + v };
            if x {
                *src.get_pixel(at(c), r)
            } else {
                *src.get_pixel(c, at(r))
            }
        });
        let name = piece
            .image
            .rsplit_once('/')
            .map_or(piece.image.as_str(), |(_, f)| f)
            .trim_end_matches(".png")
            .to_owned();
        let axes = (TileAxis::Start { offset: 0 }, TileAxis::Start { offset: 0 });
        if let Some((t, png)) = shape::tile(art.window, &name, &img, axes, true)? {
            art.files.insert(t.image.clone(), png);
            art.images.insert(t.image.clone(), img);
            let keep = u16::try_from(keep).unwrap_or(u16::MAX);
            let (width, height) = if x {
                (keep, piece.height)
            } else {
                (piece.width, keep)
            };
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

/// `a` cut for a box that lost `cut` on axis `x` (else y); `None` when its
/// normal set has nothing left to show.
fn cut_art(
    art: &mut Art<'_>,
    a: NodeArt,
    x: bool,
    cut: (u16, u16),
) -> Result<Option<NodeArt>, ImportError> {
    let normal = cut_set(art, a.normal, x, cut)?;
    let mut states = std::collections::BTreeMap::new();
    for (k, p) in a.states {
        let p = cut_set(art, p, x, cut)?;
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

/// Cuts the window's own art-carrying tiles `nodes` back inside the window
/// on each stretched axis an outdent lays past it, before
/// [`super::clip::clip`] trims their fixed axis.
///
/// # Errors
/// A cut image that does not encode, or art the tree did not encode.
pub(super) fn clip_window(nodes: &mut [SkinNode], art: &mut Art<'_>) -> Result<(), ImportError> {
    for n in nodes.iter_mut().filter(|n| n.art.is_some()) {
        for x in [true, false] {
            let o = outdent(if x { n.x } else { n.y });
            if o == (0, 0) {
                continue;
            }
            if x {
                n.x = inside(n.x);
            } else {
                n.y = inside(n.y);
            }
            if let Some(a) = n.art.take() {
                n.art = cut_art(art, a, x, o)?;
            }
            for c in &mut n.children {
                if x {
                    c.x = shift_child(c.x, o);
                } else {
                    c.y = shift_child(c.y, o);
                }
            }
        }
    }
    Ok(())
}
