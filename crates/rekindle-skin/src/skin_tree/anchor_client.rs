//! A fixed tile's client box (plan SK2-S1o, split from [`super::anchor`]
//! for plan SK17-3): the frame its children are laid in, less its
//! `Indent*`/`Outdent*`, and its children brought back to its own frame.

use rekindle_types::dto::skin_tree::{Anchor, NodeSpan, SkinNode};

use crate::import::tree::Node;
use crate::import::ImportError;

use super::anchor::{expr, i16_of, resize, Axis, Sibs, Sizes, X, Y};
use super::import::bad;

/// A fixed tile's client inset on each axis (x, y), `Indent*` less
/// `Outdent*` at each end: Xfire lays its children in its box less these
/// (plan SK2-S1o; StealthX's `XfireMenuText` `X="1" IndentLeft="27"` holds
/// `FileMenu` `ResizeX="100" OutdentLeft="27"`, `popups.xml:5-6`: the menu
/// art from the text tile's own left edge, x 2 inside the rail). A
/// stretched tile's indents move its own edges instead ([`axis`]).
pub(super) fn client_inset(
    tile: &Node,
    refs: (&Sibs, &Sizes),
) -> Result<[(i32, i32); 2], ImportError> {
    let one = |ax: &Axis| -> Result<(i32, i32), ImportError> {
        if resize(tile, ax).is_some() {
            return Ok((0, 0));
        }
        let ind = |k| expr(tile.attr_ci(k), refs, ax);
        Ok((
            ind(ax.indent[0])? - ind(ax.outdent[0])?,
            ind(ax.indent[1])? - ind(ax.outdent[1])?,
        ))
    };
    Ok([one(&X)?, one(&Y)?])
}

/// The client box of a tile laid at `span` with `inset`: its edges moved in,
/// for the names its children see ([`outer_for`]).
pub(super) fn client_box(span: NodeSpan, (s, e): (i32, i32)) -> Result<NodeSpan, ImportError> {
    // A boxless tile (no art, no declared size) has no client box: its
    // indents inset nothing (Total Chaos's, xSLIMFIRE's art-less wrappers).
    let NodeSpan::Fixed { at, size } = span.clone() else {
        return Ok(span);
    };
    if size == 0 {
        return Ok(span);
    }
    let at = match at {
        Anchor::Start(o) => Anchor::Start(i16_of(i32::from(o) + s)?),
        Anchor::End(o) => Anchor::End(i16_of(i32::from(o) + e)?),
        Anchor::After { sibling, gap } => Anchor::After {
            sibling,
            gap: i16_of(i32::from(gap) + s)?,
        },
        // The client box is the tile's box less `(s, e)`: centred in the
        // same frame, narrowed by the insets (lead decision 32).
        Anchor::Centre { by, before, after } => Anchor::Centre {
            by,
            before: i16_of(i32::from(before) - s)?,
            after: i16_of(i32::from(after) - e)?,
        },
    };
    let size = u16::try_from(i32::from(size) - s - e)
        .map_err(|_| bad(format!("a client inset {s}+{e} wider than its tile {size}")))?;
    Ok(NodeSpan::Fixed { at, size })
}

/// `child` (laid in the client box of a tile inset by `(s, e)`)
/// in the tile's own frame.
fn from_client(child: NodeSpan, (s, e): (i32, i32)) -> Result<NodeSpan, ImportError> {
    let add = |o: i16, d: i32| i16_of(i32::from(o) + d);
    Ok(match child {
        // A share of the client box is not the same share of the tile's.
        NodeSpan::Share { .. } => return Err(bad("a share inside an inset tile".to_owned())),
        NodeSpan::Stretch { start, end } => NodeSpan::Stretch {
            start: add(start, s)?,
            end: add(end, e)?,
        },
        NodeSpan::Fixed { at, size: own } => {
            let at = match at {
                Anchor::Start(o) => Anchor::Start(add(o, s)?),
                Anchor::End(o) => Anchor::End(add(o, e)?),
                // Its sibling moves with it: the gap stays.
                after @ Anchor::After { .. } => after,
                // Centred in the client box: the tile's box narrowed by
                // `(s, e)` (lead decision 32).
                Anchor::Centre { by, before, after } => Anchor::Centre {
                    by,
                    before: add(before, -s)?,
                    after: add(after, -e)?,
                },
            };
            NodeSpan::Fixed { at, size: own }
        }
    })
}

/// `children` laid in the client box of a tile at `(x, y)` inset by
/// `inset` ([`client_inset`]), in the tile's own frame.
pub(super) fn to_tile(
    mut children: Vec<SkinNode>,
    (x, y): (NodeSpan, NodeSpan),
    inset: [(i32, i32); 2],
) -> Result<Vec<SkinNode>, ImportError> {
    for (span, (s, e), c) in [(x, inset[0], 0), (y, inset[1], 1)] {
        let NodeSpan::Fixed { size, .. } = span else {
            continue;
        };
        if (s, e) == (0, 0) || size == 0 {
            continue;
        }
        for n in &mut children {
            let slot = if c == 0 { &mut n.x } else { &mut n.y };
            *slot = from_client(slot.clone(), (s, e))?;
        }
    }
    Ok(children)
}
