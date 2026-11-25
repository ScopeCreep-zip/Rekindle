//! Plan SK15 (`settle`): a target box the cuts left short of 24 px takes
//! its 24 px from the free room on its far side or a neighbour's spare
//! share of the gap, so it meets WCAG 2.2 SC 2.5.8 by size without moving
//! its art.

use rekindle_types::dto::skin_tree::TargetRule;

use super::{target, validate};
use crate::{builtin, manifest, package};

/// The served Main tree of the built-in `id` and its minimum window.
fn main_of(id: &str) -> (rekindle_types::dto::skin_tree::SkinTree, (i32, i32)) {
    let b = builtin::PACKS.iter().find(|b| b.id == id).unwrap();
    let m = manifest::served(&package::read(b.bytes).unwrap()).unwrap();
    let l = m
        .windows
        .iter()
        .find(|(w, _)| format!("{w:?}") == "Main")
        .map(|(_, l)| l)
        .unwrap();
    (l.tree.clone().unwrap(), target::window(l))
}

fn laid(tree: &rekindle_types::dto::skin_tree::SkinTree, at: (i32, i32), id: &str) -> target::Laid {
    target::targets(tree, at)
        .into_iter()
        .find(|l| l.id == id)
        .unwrap()
}

/// SpaceCowboy's `Logo` (16 x 16, its `Title` 2 px right, `ToolsMenu`
/// below; lead ruling: it keeps its `Menu(Sys)` press) and Onyx's `Logo`
/// (its `Title` 2 px right, its `StatusEdit` 2 px below) were served
/// 22 x 23 and 22 x 21 (`Short`); each now serves 24 x 24 beside its art,
/// at the minimum and at 400 x 900, clear of every other target.
#[test]
fn a_logo_beside_its_title_serves_a_24_px_target() {
    for id in ["spacecowboy", "onyx"] {
        let (tree, min) = main_of(id);
        for at in [min, (400, 900)] {
            let l = laid(&tree, at, "Logo");
            assert_eq!(l.rule, TargetRule::Size, "{id} {at:?}: {l:?}");
            assert_eq!((l.area[2], l.area[3]), (24, 24), "{id} {at:?}: {l:?}");
            assert!(validate::overlap(l.area, l.art), "{id} {at:?}: {l:?}");
            for u in target::targets(&tree, at) {
                if u.id != l.id && !validate::overlap(u.art, l.art) {
                    assert!(
                        !validate::overlap(u.area, l.area),
                        "{id} {at:?}: {u:?} {l:?}"
                    );
                }
            }
        }
    }
}

/// Simple-X's `StatusEdit` (14 px tall, its button row 5 px above) lost
/// 1 px above to the row (`[30, 231, 299, 23]`, `Short`) with 6 px of the
/// window free below; it now takes that 1 px below its art.
#[test]
fn a_status_field_cut_above_takes_its_24_px_below() {
    let (tree, min) = main_of("simple-x");
    let l = laid(&tree, min, "StatusEdit");
    assert_eq!(l.art, [30, 235, 292, 14]);
    assert_eq!(l.rule, TargetRule::Size, "{l:?}");
    assert_eq!((l.area[1], l.area[3]), (231, 24), "{l:?}");
}
