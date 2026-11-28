//! SKC4c: a `Resize*` tile laid from an edge of a sibling that moves with
//! the window chains on that sibling ([`NodeSpan::Share`] after it), and
//! the boxes and stylesheet lay it from the sibling's box at every width.

use rekindle_types::dto::skin_tree::{Anchor, NodeSpan, SkinTree};

use super::anchor_tests::imported;

/// `id`'s box in a window `w × h`.
fn box_of(tree: &SkinTree, (w, h): (i32, i32), id: &str) -> [i32; 4] {
    tree.boxes(w, h)
        .into_iter()
        .find(|(n, _)| n.id == id)
        .map(|(_, b)| b)
        .unwrap()
}

/// Bionical's chat (`biochat.ini:113-131`): `SendButton` `JustX=CENTER`,
/// `Rightlines` `X=SendButton.RIGHT ResizeX=100 IndentRight=20`, `Leftlines`
/// `X=-21 ResizeX=50`, here `CloseBox` (18 px), `MainTitleBar` and
/// `XfireLogo`.
const BIONICAL: &[u8] = br#"<Tile Name="W" Component="MainWindow">
<Tile Name="CloseBox" JustX="CENTER" JustY="BOTTOM" Component="CloseBox"/>
<Tile Name="MainTitleBar" X="CloseBox.RIGHT" JustY="BOTTOM" Y="8" ResizeX="100" IndentRight="20" Component="MainTitleBar"/>
<Tile Name="XfireLogo" X="-21" JustY="BOTTOM" Y="8" ResizeX="50" Component="XfireLogo"/></Tile>"#;

/// The right line runs from the centred button's right edge to 20 px short
/// of the window's right edge, and the left line ends before the button,
/// at two widths: the pair flanks the button as the window grows. The
/// importer refused this span before SKC4c ("ax stretch from an edge and
/// ax far side").
#[test]
fn a_stretch_from_a_centred_siblings_edge_flanks_it_at_every_width() {
    let (_, tree) = imported(BIONICAL).unwrap();
    let kids = &tree.root;
    assert_eq!(
        kids[1].x,
        NodeSpan::Share {
            at: Anchor::After {
                sibling: "CloseBox".into(),
                gap: 0,
            },
            permille: 1000,
            end: 20,
        }
    );
    // `Leftlines` hangs 21 px off the window's left edge: the window's
    // outsets hold it, and the pack's own box lies inside them.
    let (ol, or) = (i32::from(tree.outsets.left), i32::from(tree.outsets.right));
    for w in [200, 333] {
        let button = box_of(&tree, (w, 300), "CloseBox");
        let right = box_of(&tree, (w, 300), "MainTitleBar");
        let left = box_of(&tree, (w, 300), "XfireLogo");
        assert_eq!(button[0], ol + (w - ol - or - 18) / 2, "{w}");
        assert_eq!(right[0], button[0] + button[2], "{w}");
        assert_eq!(right[0] + right[2], w - or - 20, "{w}");
        assert!(left[0] + left[2] <= button[0], "{w}: {left:?} {button:?}");
    }
    let css = super::span_css::axis(&kids[1].x, &kids[..1], ("left", "right", "width"), false);
    assert!(css.contains("+ 18px + 0px)"), "{css}");
    assert!(css.contains("- 20px) * 1000 / 1000), 1px)"), "{css}");
}

/// nvidia[Reactor]'s chat (`nVidiaChatSkin.ini:56-59,92-97`):
/// `RightResizeBot` `Y=InfoViewButtonBg.BOTTOM-10 ResizeY=100` below the
/// `JustY=CENTER` `InfoViewButtonBg`: from 10 px above the button's bottom
/// to the window's bottom at two heights.
#[test]
fn a_stretch_from_a_vertically_centred_siblings_bottom_reaches_the_far_side() {
    let (_, tree) = imported(
        br#"<Tile Name="W" Component="MainWindow">
<Tile Name="CloseBox" JustX="RIGHT" JustY="CENTER" Component="CloseBox"/>
<Tile Name="XfireLogo" JustX="RIGHT" Y="CloseBox.BOTTOM-10" ResizeY="100" Component="XfireLogo"/></Tile>"#,
    )
    .unwrap();
    assert_eq!(
        tree.root[1].y,
        NodeSpan::Share {
            at: Anchor::After {
                sibling: "CloseBox".into(),
                gap: -10,
            },
            permille: 1000,
            end: 0,
        }
    );
    for h in [300, 451] {
        let button = box_of(&tree, (200, h), "CloseBox");
        let grip = box_of(&tree, (200, h), "XfireLogo");
        assert_eq!(button[1], (h - 16) / 2, "{h}");
        assert_eq!(grip[1], button[1] + button[3] - 10, "{h}");
        assert_eq!(grip[1] + grip[3], h, "{h}");
    }
}

/// A share from the top edge of a bottom-anchored (not centred) sibling
/// moves with the window too: half of what lies between that edge and the
/// far side. Refused before SKC4c ("a share placed from a point that
/// moves").
#[test]
fn a_share_from_an_end_anchored_siblings_near_edge_moves_with_it() {
    let (_, tree) = imported(
        br#"<Tile Name="W" Component="MainWindow">
<Tile Name="CloseBox" JustY="BOTTOM" Y="4" Component="CloseBox"/>
<Tile Name="XfireLogo" Y="CloseBox.TOP" ResizeY="50" Component="XfireLogo"/></Tile>"#,
    )
    .unwrap();
    assert_eq!(
        tree.root[1].y,
        NodeSpan::Share {
            at: Anchor::After {
                sibling: "CloseBox".into(),
                gap: -16,
            },
            permille: 500,
            end: 0,
        }
    );
    for h in [300, 451] {
        let button = box_of(&tree, (200, h), "CloseBox");
        let half = box_of(&tree, (200, h), "XfireLogo");
        assert_eq!(half[1], button[1], "{h}");
        assert_eq!(half[3], 10, "{h}");
    }
}

/// A stretch justified to its far side from a moving edge contradicts
/// itself (both ends would be measured from the far side) and stays the
/// importer's error.
#[test]
fn a_right_justified_stretch_from_a_centred_edge_is_refused() {
    let e = imported(
        br#"<Tile Name="W" Component="MainWindow">
<Tile Name="CloseBox" JustX="CENTER" Component="CloseBox"/>
<Tile Name="MainTitleBar" JustX="RIGHT" X="CloseBox.RIGHT" ResizeX="100" Component="MainTitleBar"/></Tile>"#,
    )
    .unwrap_err();
    assert!(
        e.to_string()
            .contains("ax stretch from an edge and ax far side"),
        "{e}"
    );
}
