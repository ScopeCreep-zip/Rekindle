//! Where a pack popup sits beside the window that launched it (plan
//! SK2-A5): the served [`ShellPopup`] placement applied to the launcher's
//! frame. Pure, so the rule is tested without a window.

use rekindle_types::dto::skin_layout::PopupEdge;
use rekindle_types::dto::skin_tree::ShellPopup;

/// A window's box in whole logical px on the desktop (tauri's `Pixel`
/// conversion rounds): its outer position and its inner size. Rekindle's windows are frameless (`decorations(false)`), so
/// the outer and inner boxes coincide and the popup's `position` (outer)
/// and `inner_size` take the same numbers the launcher reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowBox {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// The popup's frame beside `opener`: its left side `anchor.offset` px from
/// the launcher's anchored edge, its width the popup's own (never under
/// its minimum), and its vertical span laid in the launcher's height by
/// the same [`rekindle_types::dto::skin_tree::NodeSpan::place`] the tree
/// uses (Symbiosis's `ResizeY="100" IndentTop="23" IndentBottom="23"`),
/// never shorter than its minimum height.
#[must_use]
pub fn popup_frame(popup: &ShellPopup, opener: WindowBox) -> WindowBox {
    let offset = i32::from(popup.anchor.offset);
    let x = match popup.anchor.edge {
        PopupEdge::Left => opener.x + offset,
        PopupEdge::Right => opener.x + opener.width + offset,
    };
    let (start, size) = popup.y.place(opener.height);
    WindowBox {
        x,
        y: opener.y + start,
        width: i32::from(popup.width.max(popup.min_width)),
        height: size.max(i32::from(popup.min_height)),
    }
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
}
