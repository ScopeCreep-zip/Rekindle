//! A tile's proportional box on one axis (plan SK17-3, lead ruling):
//! Xfire's `Resize*` below 100 is that percent of the parent's length, and
//! a tile on the far edge of such a box chains on it. Combat's chat
//! `[ContentArea]` (`CombatChatSkin.ini:291-310`): `ChatArea` `Y=0
//! ResizeY=80` is 80% of the area, `Divider` `Y=ChatArea.BOTTOM` sits on
//! its far edge, `ChatEdit` `Y=Divider.BOTTOM ResizeY=100 IndentBottom=0`
//! fills the rest. A share or stretch placed at an edge of a fixed sibling
//! that moves with the window chains on that sibling too (SKC4c,
//! [`moving`]). A capped share, or one placed from any other point
//! measured from the far edge or the centre, has no Xfire pack and no
//! meaning here: an import error, never a guess.

use rekindle_types::dto::skin_tree::{Anchor, NodeSpan};

use crate::import::tree::Node;
use crate::import::xfire_xml::title::reference;
use crate::import::ImportError;

use super::anchor::{i16_of, Axis, Pos, Sibs};
use super::import::bad;
use super::sibs::span_of;

/// The share `pct` percent of the parent laid from `pos`, with the start
/// and end insets and the component's cap: one laid from its start is
/// that share of the box between its start and its end inset (plan SKW3:
/// xfire-classic's chat `ChatArea`, `ResizeY=80 IndentBottom="19+20"`).
pub(super) fn share(
    pos: Pos,
    pct: u16,
    (start_in, end_in, max): (i32, i32, Option<u16>),
    (at_end, centred): (bool, bool),
    ax: &Axis,
) -> Result<NodeSpan, ImportError> {
    if max.is_some() {
        return Err(bad(format!("{}: a capped share", ax.pos())));
    }
    let (at, end) = match (pos, at_end, centred) {
        (Pos::Num(p), _, true) => (Anchor::centre(i16_of(p)?), 0),
        (Pos::Num(p), true, false) => (Anchor::End(i16_of(p + end_in)?), 0),
        (Pos::Num(p) | Pos::Ref(super::anchor::At::Start(p)), false, false) => {
            (Anchor::Start(i16_of(p + start_in)?), i16_of(end_in)?)
        }
        _ => {
            return Err(bad(format!(
                "{}: a share placed from a point that moves",
                ax.pos()
            )))
        }
    };
    Ok(NodeSpan::Share {
        at,
        permille: pct * 10,
        end,
    })
}

/// The sibling and gap a `Resize*` tile placed at an edge of a fixed
/// sibling that moves with the window chains on (SKC4c): its far edge
/// `+n` is `n` past it, its near edge `+n` is `n` less its size past it.
/// Bionical's chat `Rightlines` `X=SendButton.RIGHT ResizeX=100
/// IndentRight=20` beside the centred `SendButton` (`biochat.ini:113-124`)
/// runs from the button's right edge to 20 px short of the window's, as
/// the pair of lines flanks the button at every width; nvidia[Reactor]'s
/// `RightResizeBot` `Y=InfoViewButtonBg.BOTTOM Y=-10 ResizeY=100` below the
/// `JustY=CENTER` button (`nVidiaChatSkin.ini:56-59,92-97`). `None` for a
/// sibling's `CENTER`, a sibling laid in another parent (its box is not in
/// this level for `SkinTree::boxes` to place from) or one not fixed at
/// the end or the centre.
pub(super) fn moving(tile: &Node, ax: &Axis, sibs: &Sibs) -> Option<(String, i32)> {
    let (name, side, by) = reference(tile.attr_ci(ax.pos())?)?;
    if !sibs.laid_here(name) {
        return None;
    }
    let NodeSpan::Fixed {
        at: Anchor::End(_) | Anchor::Centre { .. },
        size,
    } = span_of(sibs, name, ax)?
    else {
        return None;
    };
    let gap = if side.eq_ignore_ascii_case(ax.far) {
        by
    } else if side.eq_ignore_ascii_case(ax.near) {
        by - i32::from(size)
    } else {
        return None;
    };
    Some((name.trim().to_owned(), gap))
}

/// A tile `by` px past the far edge of `sibling` (a share, or a tile
/// chained past one), of its own `size`, or stretched (`resize` 100) or a
/// share of what remains between its start and its end inset (plan SKW3,
/// lead ruling b: xfire-classic's chat `ChatEdit`, `Y=Divider.BOTTOM
/// ResizeY=100 IndentBottom=19`, `xfirechatwindow.xml:211-238`, stops 19 px
/// short of its area's far edge).
pub(super) fn chained(
    (sibling, by): (String, i32),
    size: u16,
    resize: Option<u16>,
    (start_in, end_in, max): (i32, i32, Option<u16>),
    (at_end, centred): (bool, bool),
    ax: &Axis,
) -> Result<NodeSpan, ImportError> {
    if at_end || centred || max.is_some() {
        return Err(bad(format!(
            "{}: a chain on {sibling} justified or capped",
            ax.pos()
        )));
    }
    let at = Anchor::After {
        sibling,
        gap: i16_of(by + start_in)?,
    };
    match resize {
        None => Ok(NodeSpan::Fixed { at, size }),
        Some(pct) => Ok(NodeSpan::Share {
            at,
            permille: pct * 10,
            end: i16_of(end_in)?,
        }),
    }
}
