//! The ad's box (plan E7.7, CLAUDE.md "Key Rules": the Rekindle banner
//! takes the place of Xfire's served ad). The shell draws its banner over
//! the node with role `AdArea`, so that node is the pack's `Class="AdView"`
//! tile, never the container around it, and its box lies inside the
//! window's silhouette: StealthX's full-width `AdArea` container showed the
//! banner's ends beside the friends view, and Total Chaos's 470 px `AdBar`
//! painted it over the frame's transparent edges.

use rekindle_types::dto::skin::SkinWindow;
use rekindle_types::dto::skin_tree::{NodeRole, SkinTree};

use super::*;
use crate::{builtin, images, manifest, package};

/// Each built-in's ad: the tile whose component is `Class="AdView"` in
/// its source (`wt/skinsrc` `legacy/skins/`).
const AD_VIEWS: [(&str, &str); 11] = [
    // `Components/MainWindow.xml:191`, a top-level tile.
    ("symbiosis", "AdView"),
    // The embedded skin's `AdArea` > `AdView`.
    ("xfire-classic", "AdView"),
    // `combat.ini:279-280` `[AdArea]` `Class=AdView`, no container.
    ("combat", "AdArea"),
    // `Alloy.ini:638-650`: `[AdArea]` `Class=Tile` art > `[AdView]`.
    ("alloy", "AdView"),
    // `components.xml:346-347`, inside `AdBG` art (`mainwindow.xml:68-69`).
    ("onyx", "AdArea"),
    // `SimpleX.ini:729-741`: `[AdArea]` `Class=Tile` art > `[AdView]`.
    ("simple-x", "AdView"),
    // `components.xml:250-251`, inside `AdAreaBG` art (`mainwindow.xml:12-13`).
    ("spacecowboy", "AdArea"),
    // `components.xml:637-638`, `mainwindow.xml:16-17`.
    ("stealthx", "AdView"),
    // `components.xml:359-360`, inside `AdBar` art (`mainwindow.xml:11-12`).
    ("total-chaos", "AdArea"),
    // `Xglow.ini:748-749` `[AdArea]` `Class=AdView`, no container.
    ("xglow", "AdArea"),
    // `xSLIMFIREcomponents.xml:77-92`, `xSLIMFIREmainwindow.xml:62-63`.
    ("xslimfire", "AdView"),
];

/// Where row `y..y + h` of box `[x, y, w, h]` first leaves `rects`, if it
/// does.
fn outside(rects: &[Rect], [x, y, w, h]: [i32; 4]) -> Option<String> {
    for row in i64::from(y)..i64::from(y) + i64::from(h) {
        let mut runs: Vec<(i64, i64)> = rects
            .iter()
            .filter(|r| row >= i64::from(r.y) && row < i64::from(r.y) + i64::from(r.height))
            .map(|r| (i64::from(r.x), i64::from(r.x) + i64::from(r.width)))
            .collect();
        runs.sort_unstable();
        let mut at = i64::from(x);
        for (a, b) in runs {
            if a <= at && b > at {
                at = b;
            }
        }
        if at < i64::from(x) + i64::from(w) {
            return Some(format!("row {row} is opaque only to x {at}"));
        }
    }
    None
}

/// The ids and boxes of the nodes with role `AdArea` in `tree` at `w × h`.
fn ads(tree: &SkinTree, w: u32, h: u32) -> Vec<(String, [i32; 4])> {
    let (w, h) = (i32::try_from(w).unwrap(), i32::try_from(h).unwrap());
    tree.boxes(w, h)
        .into_iter()
        .filter(|(n, _)| n.role == NodeRole::AdArea)
        .map(|(n, b)| (n.id.clone(), b))
        .collect()
}

#[test]
fn every_builtin_ad_is_its_ad_view_inside_the_silhouette() {
    let mut wrong = Vec::new();
    for b in &builtin::PACKS {
        let package = package::read(b.bytes).unwrap();
        let m = manifest::served(&package).unwrap();
        let files = images::validate(&package, &m).unwrap();
        for (window, layout) in &m.windows {
            let Some(tree) = &layout.tree else { continue };
            let masks = of_layout("windows", layout, &files).unwrap();
            let o = layout.outsets;
            let min = (
                u32::from(layout.min_width) + u32::from(o.left) + u32::from(o.right),
                u32::from(layout.min_height) + u32::from(o.top) + u32::from(o.bottom),
            );
            for (w, h) in [min, (min.0 + 300, min.1 + 300)] {
                let rects = silhouette(layout, &masks, w, h, SCALE_ONE).unwrap();
                let found = ads(tree, w, h);
                if *window == SkinWindow::Main {
                    let want = AD_VIEWS.iter().find(|(p, _)| *p == b.id).unwrap().1;
                    let ids: Vec<&str> = found.iter().map(|(id, _)| id.as_str()).collect();
                    if ids != [want] {
                        wrong.push(format!("{} main {w}x{h}: ad {ids:?}, want [{want}]", b.id));
                    }
                }
                for (id, at) in &found {
                    if let Some(e) = outside(&rects, *at) {
                        wrong.push(format!("{} {window:?} {w}x{h}: {id} {at:?}: {e}", b.id));
                    }
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn stealthx_ad_is_234x60_centred_in_its_container() {
    let (_, b) = builtin::find("builtin:stealthx").unwrap();
    let package = package::read(b.bytes).unwrap();
    let mut m = manifest::served(&package).unwrap();
    let layout = m.windows.remove(&SkinWindow::Main).unwrap();
    let tree = layout.tree.unwrap();
    for (w, h) in [(400, 555), (700, 855)] {
        let boxes = tree.boxes(w, h);
        let at = |id: &str| boxes.iter().find(|(n, _)| n.id == id).unwrap();
        let (ad, [x, y, aw, ah]) = at("AdView");
        let (_, [cx, cy, cw, _]) = at("AdArea");
        assert_eq!(ad.role, NodeRole::AdArea, "{w}x{h}");
        assert_eq!((*aw, *ah), (234, 60), "{w}x{h}");
        assert_eq!(*y, *cy, "{w}x{h}");
        assert!(
            (x - cx - (cx + cw - x - aw)).abs() <= 1,
            "{w}x{h}: {x} in {cx}+{cw}"
        );
    }
}
