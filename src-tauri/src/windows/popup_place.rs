//! Where a pack popup sits beside the window that launched it (plan
//! SK2-A5): the served [`ShellPopup`] placement applied to the launcher's
//! frame, kept beside it as the launcher moves, the user resizes the popup
//! or the skin changes. Pure, so every rule is tested without a window.

use rekindle_types::dto::skin_layout::{PopupEdge, SkinWindow};
use rekindle_types::dto::skin_tree::{ShellPopup, SkinShell};

/// A window's box in whole logical px on the desktop (tauri's `Pixel`
/// conversion rounds): its outer position and its inner size. Rekindle's
/// windows are frameless (`decorations(false)`), so the outer and inner
/// boxes coincide and the popup's `position` (outer) and `inner_size` take
/// the same numbers the launcher reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowBox {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// An open popup: its served placement and the size the shell last laid
/// it at. A current size other than `laid` is the user's resize (the
/// pack's resize tiles, `ResizeR`), which following keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenPopup {
    pub place: ShellPopup,
    pub laid: (i32, i32),
}

/// The popup's laid size beside `opener`: its own width (never under its
/// minimum), and the length of its vertical span laid in the launcher's
/// height by the same [`rekindle_types::dto::skin_tree::NodeSpan::place`]
/// the tree uses (Symbiosis's `ResizeY="100" IndentTop="23"
/// IndentBottom="23"`), never shorter than its minimum height.
#[must_use]
pub fn popup_size(popup: &ShellPopup, opener: WindowBox) -> (i32, i32) {
    let (_, size) = popup.y.place(opener.height);
    (
        i32::from(popup.width.max(popup.min_width)),
        size.max(i32::from(popup.min_height)),
    )
}

/// The popup's position beside `opener` at its current `width`: its left
/// side `anchor.offset` px from the launcher's anchored edge, its top the
/// start of its vertical span. A left-anchored popup that sits wholly left
/// of its launcher at its laid width (offset + laid width <= 0) keeps its
/// right side where that layout put it, so a wider or narrower popup still
/// meets its launcher; every other anchor fixes the left side, as the
/// pack's `X` does.
#[must_use]
pub fn popup_position(popup: &ShellPopup, opener: WindowBox, width: i32) -> (i32, i32) {
    let offset = i32::from(popup.anchor.offset);
    let laid_width = i32::from(popup.width.max(popup.min_width));
    let x = match popup.anchor.edge {
        PopupEdge::Left if offset + laid_width <= 0 => opener.x + offset + laid_width - width,
        PopupEdge::Left => opener.x + offset,
        PopupEdge::Right => opener.x + opener.width + offset,
    };
    let (start, _) = popup.y.place(opener.height);
    (x, opener.y + start)
}

/// The popup's frame as first laid beside `opener` (opening, a new skin).
#[must_use]
pub fn popup_frame(popup: &ShellPopup, opener: WindowBox) -> WindowBox {
    let (width, height) = popup_size(popup, opener);
    let (x, y) = popup_position(popup, opener, width);
    WindowBox {
        x,
        y,
        width,
        height,
    }
}

/// The popup's frame when its launcher moved or resized and the popup is
/// `current` (width, height) now: laid afresh while it is the size the
/// shell laid, else (the user resized it) at its current size, with only
/// its position recomputed.
#[must_use]
pub fn follow_frame(open: &OpenPopup, opener: WindowBox, current: (i32, i32)) -> WindowBox {
    if current == open.laid {
        return popup_frame(&open.place, opener);
    }
    let (x, y) = popup_position(&open.place, opener, current.0);
    WindowBox {
        x,
        y,
        width: current.0,
        height: current.1,
    }
}

/// The popup `name` a launcher drawn with `outline` has in a skin's served
/// shells, or `None`: after a skin change, `None` closes the open popup.
#[must_use]
pub fn served_popup(shells: &[SkinShell], outline: SkinWindow, name: &str) -> Option<ShellPopup> {
    shells
        .iter()
        .filter(|shell| shell.window == outline)
        .find_map(|shell| shell.popup.as_ref().filter(|p| p.name == name))
        .cloned()
}

#[cfg(test)]
#[path = "popup_place_tests.rs"]
mod tests;
