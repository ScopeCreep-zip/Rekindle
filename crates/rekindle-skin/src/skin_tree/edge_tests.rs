//! Plan E7.7 TAB1 (owner rule): the app takes the whole window and an edge
//! node lives on the frame's edge, never in a transparent strip the window
//! reserves: Symbiosis's InfoView tab `MainInfoViewButton` on its frame's
//! right border, and every built-in's served nodes inside its frame.

use std::collections::BTreeMap;

use rekindle_types::dto::skin::{Insets, PopupEdge, SkinWindow, WindowLayout};
use rekindle_types::dto::skin_tree::{SkinNode, SkinTree, TargetRule};

use super::import;
use super::test_support::{frame, piece, Z};
use super::validate::shown;
use crate::import::tree::parse;
use crate::{builtin, manifest, package};

fn box_of(boxes: &[(&SkinNode, [i32; 4])], id: &str) -> [i32; 4] {
    boxes.iter().find(|(n, _)| n.id == id).unwrap().1
}

/// Symbiosis's frame (`MainWindow.xml:35-39`): rows `IndentRight="13"`, the
/// 13 px `RightSide` border `X="13" JustX="Right"`, and the tab
/// `JustX="Right" JustY="Center"` in the 13 px column (`:199`). The strip is
/// given up: the frame fills the window, the tab sits on its right border,
/// over `RightSide`, its art whole, and every node lies in the window.
#[test]
fn an_edge_tab_sits_on_the_frames_edge_not_in_a_strip() {
    let main = parse(
        "main.xml",
        br#"<Tile Name="W" Component="MainWindow">
<Tile Name="TopRow" Z="1" ResizeX="100" IndentRight="13" Component="TopRow"/>
<Tile Name="Background" Z="1" Y="13" ResizeX="100" ResizeY="100" IndentLeft="13" IndentRight="26" IndentBottom="13" Component="Bg"/>
<Tile Name="RightSide" X="13" Y="13" Z="1" JustX="Right" ResizeY="100" IndentBottom="13" Component="Side"/>
<Tile Name="BottomRow" Z="1" JustY="Bottom" ResizeX="100" IndentRight="13" Component="BottomRow"/>
<Tile Name="Tab" Z="30" JustX="Right" JustY="Center" Component="Tab"/>
</Tile>"#,
    )
    .unwrap();
    let parts = parse(
        "parts.xml",
        br#"<Components>
<Component Name="MainWindow" MinWidth="100" MinHeight="200"/>
<Component Name="TopRow"><Images State="Normal"><Image Type="Middle">top-10x13</Image></Images></Component>
<Component Name="BottomRow"><Images State="Normal"><Image Type="Middle">bot-10x13</Image></Images></Component>
<Component Name="Bg"><Images State="Normal"><Image Type="Middle">bg-10x10</Image></Images></Component>
<Component Name="Side"><Images State="Normal"><Image Type="Middle">side-13x10</Image></Images></Component>
<Component Name="Tab"><Images State="Normal"><Image Type="Middle">tab-13x115</Image></Images></Component>
</Components>"#,
    )
    .unwrap();
    let mut built = (frame(), BTreeMap::new());
    let rows = ["TopRow".to_owned(), "BottomRow".to_owned()];
    import::add(
        SkinWindow::Main,
        &main,
        std::slice::from_ref(&parts),
        |path, _| Ok(piece(path)),
        &rows,
        &mut built,
    )
    .unwrap();
    let (layout, tree) = (&built.0, built.0.tree.as_ref().unwrap());
    assert_eq!(tree.outsets, Z, "the tree fills the window");
    assert_eq!(layout.outsets, Z, "the frame fills the window: no strip");
    let (w, h) = (i32::from(layout.min_width), i32::from(layout.min_height));
    let boxes = tree.boxes(w, h);
    assert_eq!(box_of(&boxes, "TopRow"), [0, 0, w, 13]);
    let side = box_of(&boxes, "RightSide");
    assert_eq!((side[0], side[2]), (w - 13, 13));
    let tab = box_of(&boxes, "Tab");
    assert_eq!(tab[0] + tab[2], w, "the tab's outer edge is the frame's");
    assert_eq!(
        (tab[0], tab[2], tab[3]),
        (side[0], 13, 115),
        "on the border, art whole"
    );
    assert_eq!(tab[1], (h - 115).div_euclid(2), "still centred on the edge");
    for (n, b) in &boxes {
        assert!(
            b[0] >= 0 && b[1] >= 0 && b[0] + b[2] <= w && b[1] + b[3] <= h,
            "{} {b:?}",
            n.id
        );
    }
}

fn served(id: &str) -> BTreeMap<SkinWindow, WindowLayout> {
    let (_, b) = builtin::find(&format!("builtin:{id}")).unwrap();
    manifest::served(&package::read(b.bytes).unwrap())
        .unwrap()
        .windows
}

/// The frame box of `layout` at its smallest window, and that window.
fn frame_box(layout: &WindowLayout) -> ([i32; 4], i32, i32) {
    let out = layout.outsets;
    let wide = i32::from(layout.min_width) + i32::from(out.left) + i32::from(out.right);
    let high = i32::from(layout.min_height) + i32::from(out.top) + i32::from(out.bottom);
    let frame = [
        i32::from(out.left),
        i32::from(out.top),
        wide - i32::from(out.right),
        high - i32::from(out.bottom),
    ];
    (frame, wide, high)
}

/// Every built-in window's served tree fills its window (`outsets` zero),
/// every node with art lies inside the frame box, and the frame fills the
/// window except where a pack's frame leaves room no node's art reaches
/// into: StealthX's main `MainBackground` `X="112"`, the room its
/// `MainMenuPopup` rail opens in (plan SK2-S1o), a popup and not an edge node.
#[test]
fn every_builtin_lays_its_nodes_in_its_frame() {
    let ids = [
        "symbiosis",
        "xfire-classic",
        "combat",
        "alloy",
        "onyx",
        "simple-x",
        "spacecowboy",
        "stealthx",
        "total-chaos",
        "xglow",
        "xslimfire",
    ];
    let room = |left, top, right, bottom| Insets {
        top,
        right,
        bottom,
        left,
    };
    let exceptions = [("stealthx", SkinWindow::Main, room(112, 0, 0, 0))];
    let mut wrong = Vec::new();
    for id in ids {
        for (window, layout) in served(id) {
            let Some(tree) = layout.tree.as_ref() else {
                continue;
            };
            let want = exceptions
                .iter()
                .find(|(i, w, _)| *i == id && *w == window)
                .map_or(Z, |e| e.2);
            if tree.outsets != Z || layout.outsets != want {
                wrong.push(format!(
                    "{id} {window:?} tree {:?} frame {:?}",
                    tree.outsets, layout.outsets
                ));
            }
            let (frame, w, h) = frame_box(&layout);
            for (n, b) in SkinTree::boxes(tree, w, h) {
                let drawn = n.art.is_some() && shown(n) && b[2] > 0 && b[3] > 0;
                if drawn
                    && (b[0] < frame[0]
                        || b[1] < frame[1]
                        || b[0] + b[2] > frame[2]
                        || b[1] + b[3] > frame[3])
                {
                    wrong.push(format!("{id} {window:?} {} {b:?} outside {frame:?}", n.id));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Symbiosis's InfoView tab touches the frame's right edge, which is the
/// window's, and its panel (`MainInfoViewPopup`, `Popups.xml:10`) opens
/// from that edge.
#[test]
fn the_symbiosis_infoview_tab_and_panel_are_on_the_frames_right_edge() {
    let layout = served("symbiosis").remove(&SkinWindow::Main).unwrap();
    assert_eq!(layout.outsets, Z);
    let (frame, w, h) = frame_box(&layout);
    let tree = layout.tree.as_ref().unwrap();
    let boxes = tree.boxes(w, h);
    let tab = box_of(&boxes, "MainInfoViewButton");
    assert_eq!(tab[0] + tab[2], frame[2], "{tab:?} in frame {frame:?}");
    let side = box_of(&boxes, "RightSide");
    assert_eq!(
        side[0] + side[2],
        frame[2],
        "the border reaches the window'status edge"
    );
    // Lead ruling: the tab'status target keeps its whole 115 px span in its
    // column, over the list no further than its art (plan SK12), meeting
    // the spacing exception; `StatusChanger` beside the window'status edge still
    // meets 24 x 24.
    let node = |id: &str| boxes.iter().find(|(n, _)| n.id == id).unwrap().0;
    let tab_t = node("MainInfoViewButton").target.unwrap();
    assert_eq!(
        (tab_t.area[2], tab_t.area[3], tab_t.rule),
        (13, 115, TargetRule::Spacing),
        "{tab_t:?}"
    );
    let status = node("StatusChanger").target.unwrap();
    assert_eq!(status.rule, TargetRule::Size, "{status:?}");
    let panel = &layout.popups["MainInfoViewPopup"].anchor;
    assert_eq!((panel.edge, panel.offset), (PopupEdge::Right, 0));
}
