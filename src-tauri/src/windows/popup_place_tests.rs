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
        menus: rekindle_types::dto::skin_action::MenuRegistry { menus: Vec::new() },
        min_width: 0,
        min_height: 0,
        max_width: None,
        max_height: None,
        images: Vec::new(),
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
