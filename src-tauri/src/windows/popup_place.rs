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
mod tests {
    use super::*;
    use rekindle_types::dto::skin_layout::PopupAnchor;
    use rekindle_types::dto::skin_tree::{Anchor, NodeSpan};

    const OPENER: WindowBox = WindowBox {
        x: 100,
        y: 50,
        width: 300,
        height: 600,
    };

    fn popup(edge: PopupEdge, offset: i16, y: NodeSpan) -> ShellPopup {
        ShellPopup {
            name: "Info".to_owned(),
            anchor: PopupAnchor { edge, offset },
            width: 200,
            min_width: 150,
            min_height: 100,
            y,
        }
    }

    const STRETCH: NodeSpan = NodeSpan::Stretch { start: 23, end: 23 };

    #[test]
    fn a_right_anchor_starts_from_the_launchers_right_edge() {
        let f = popup_frame(&popup(PopupEdge::Right, -13, STRETCH), OPENER);
        assert_eq!(f.x, 387);
    }

    #[test]
    fn a_left_anchor_starts_from_the_launchers_left_edge() {
        let f = popup_frame(&popup(PopupEdge::Left, -200, STRETCH), OPENER);
        assert_eq!(f.x, -100);
    }

    #[test]
    fn a_stretched_span_follows_the_launchers_height() {
        let f = popup_frame(&popup(PopupEdge::Right, 0, STRETCH), OPENER);
        assert_eq!((f.y, f.height), (73, 554));
        let taller = WindowBox {
            height: 700,
            ..OPENER
        };
        assert_eq!(
            popup_frame(&popup(PopupEdge::Right, 0, STRETCH), taller).height,
            654
        );
    }

    #[test]
    fn a_fixed_span_keeps_its_size_at_its_anchor() {
        let y = NodeSpan::Fixed {
            at: Anchor::End(10),
            size: 240,
        };
        let f = popup_frame(&popup(PopupEdge::Right, 0, y), OPENER);
        assert_eq!((f.y, f.height), (50 + 350, 240));
    }

    #[test]
    fn the_minimums_hold() {
        let mut narrow = popup(
            PopupEdge::Right,
            0,
            NodeSpan::Stretch {
                start: 290,
                end: 290,
            },
        );
        narrow.width = 80;
        let f = popup_frame(&narrow, OPENER);
        assert_eq!((f.width, f.height), (150, 100));
        let wide = popup_frame(&popup(PopupEdge::Right, 0, STRETCH), OPENER);
        assert_eq!(wide.width, 200);
    }

    fn opened(place: ShellPopup) -> OpenPopup {
        let (w, h) = popup_size(&place, OPENER);
        OpenPopup {
            place,
            laid: (w, h),
        }
    }

    #[test]
    fn following_keeps_a_user_resized_popups_size() {
        let open = opened(popup(PopupEdge::Right, -13, STRETCH));
        let moved = WindowBox {
            x: 160,
            height: 700,
            ..OPENER
        };
        let f = follow_frame(&open, moved, (320, 410));
        assert_eq!((f.width, f.height), (320, 410));
        assert_eq!((f.x, f.y), (160 + 300 - 13, 73));
    }

    #[test]
    fn following_relays_a_popup_the_user_left_alone() {
        let open = opened(popup(PopupEdge::Right, -13, STRETCH));
        assert_eq!(open.laid, (200, 554));
        let taller = WindowBox {
            height: 700,
            ..OPENER
        };
        let f = follow_frame(&open, taller, open.laid);
        assert_eq!((f.width, f.height), (200, 654));
    }

    #[test]
    fn a_popup_left_of_its_launcher_keeps_meeting_it_at_any_width() {
        let open = opened(popup(PopupEdge::Left, -200, STRETCH));
        let f = follow_frame(&open, OPENER, (260, 554));
        assert_eq!(f.x + f.width, OPENER.x);
        let over = opened(popup(PopupEdge::Left, 20, STRETCH));
        assert_eq!(follow_frame(&over, OPENER, (260, 554)).x, OPENER.x + 20);
    }

    #[test]
    fn a_skin_without_the_popup_closes_it() {
        use rekindle_types::dto::skin_layout::SkinWindow;
        use rekindle_types::dto::skin_tree::SkinShell;
        let shell = |window, popup: Option<ShellPopup>| SkinShell {
            window,
            popup,
            root: Vec::new(),
        };
        let info = popup(PopupEdge::Right, -13, STRETCH);
        let shells = vec![
            shell(SkinWindow::Main, None),
            shell(SkinWindow::Main, Some(info.clone())),
        ];
        assert_eq!(served_popup(&shells, SkinWindow::Main, "Info"), Some(info));
        assert_eq!(served_popup(&shells, SkinWindow::Main, "Menu"), None);
        assert_eq!(served_popup(&shells, SkinWindow::Chat, "Info"), None);
    }
}
