//! A tile's proportional box on one axis (plan SK17-3, lead ruling):
//! Xfire's `Resize*` below 100 is that percent of the parent's length, and
//! a tile on the far edge of such a box chains on it. Combat's chat
//! `[ContentArea]` (`CombatChatSkin.ini:291-310`): `ChatArea` `Y=0
//! ResizeY=80` is 80% of the area, `Divider` `Y=ChatArea.BOTTOM` sits on
//! its far edge, `ChatEdit` `Y=Divider.BOTTOM ResizeY=100 IndentBottom=0`
//! fills the rest. A capped share, or one placed from a point measured
//! from the far edge or the centre, has no Xfire pack and no meaning here:
//! an import error, never a guess.

use rekindle_types::dto::skin_tree::{Anchor, NodeSpan};

use crate::import::ImportError;

use super::anchor::{i16_of, Axis, Pos};
use super::import::bad;

/// The share `pct` percent of the parent laid from `pos`, with the start
/// and end insets and the component's cap.
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
    let at = match (pos, at_end, centred) {
        (Pos::Num(p), _, true) => Anchor::centre(i16_of(p)?),
        (Pos::Num(p), true, false) => Anchor::End(i16_of(p + end_in)?),
        (Pos::Num(p) | Pos::Ref(super::anchor::At::Start(p)), false, false) => {
            Anchor::Start(i16_of(p + start_in)?)
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
    })
}

/// A tile `by` px past the far edge of `sibling` (a share, or a tile
/// chained past one), of its own `size`, or stretched (`resize` 100) or a
/// share of what remains past its start.
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
        // A stretch past a moving edge reaches the parent's far edge: the
        // whole of what remains, so it holds no end inset.
        Some(100) if end_in != 0 => Err(bad(format!(
            "{}: a stretch past a moving edge with an end inset",
            ax.pos()
        ))),
        Some(pct) => Ok(NodeSpan::Share {
            at,
            permille: pct * 10,
        }),
    }
}
