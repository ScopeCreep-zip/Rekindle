//! The window frame's border box (plan E7.7.7): a framed slot's element
//! gets a border as wide as its image's nine-slice insets, so the frame
//! paints in the border and the window's content sits inside the skin's
//! edge.

use rekindle_types::dto::skin::RegionFit;

use super::property;
use crate::regions::slots::RegionSlot;

/// The widest inset a framed slot's image may have to be a window edge:
/// Symbiosis's nine frame pieces are 13 px (`ChatWindow.xml` frame1_*),
/// while an Xfire INI `[OuterFrame]` is a whole-window bitmap whose insets
/// (Combat 214/78/113/79) are corner art, not an edge; a 32 px edge still
/// leaves the narrowest buddy list (300 px, `ui-skin.md`) 236 px of content.
/// A wider frame keeps the window's 1 px border and paints behind it.
const FRAME_EDGE_MAX: u16 = 32;

/// The custom property holding a framed slot's border widths: its
/// nine-slice insets (`Tpx Rpx Bpx Lpx`), set by `skin_css`.
#[must_use]
pub fn border_property(slot: &str) -> String {
    format!("{}-border", property(slot, None))
}

/// The border declarations of `slot`'s app rule; empty unless it is framed.
/// Without a skin, the 1 px border `.app-frame` has (`00-rules.css:117`).
pub(super) fn rule(slot: &RegionSlot) -> String {
    if !slot.framed {
        return String::new();
    }
    format!(
        "    border-style: solid;\n    border-width: var({}, 1px);\n",
        border_property(slot.id)
    )
}

/// The skin declaration of framed slot `id`'s border widths for an image
/// under `fit`; empty when the image is no edge (not a nine-slice, or an
/// inset wider than [`FRAME_EDGE_MAX`]).
pub(super) fn border_decl(id: &str, fit: RegionFit) -> String {
    match fit {
        RegionFit::NineSlice(i)
            if [i.top, i.right, i.bottom, i.left]
                .iter()
                .all(|v| *v <= FRAME_EDGE_MAX) =>
        {
            format!(
                "    {}: {}px {}px {}px {}px;\n",
                border_property(id),
                i.top,
                i.right,
                i.bottom,
                i.left
            )
        }
        _ => String::new(),
    }
}
