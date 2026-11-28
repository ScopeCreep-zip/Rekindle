//! The lead's SK17 ruling on a grip over a frame strip: as in Windows,
//! where a right-click on any part of the non-client frame, borders
//! included, opens the system menu, a grip whose band meets a strip whose
//! only gesture is the right-click system menu (Symbiosis's `ChatTopRow`,
//! `OnRightClick ContextMenuSys`) is not cut by it and serves that
//! right-click itself; a strip with any other trigger still cuts it.

use rekindle_types::dto::skin::WindowLayout;
use rekindle_types::dto::skin_action::{EngineMenu, NodeTrigger, SkinAction, SkinEvent};
use rekindle_types::dto::skin_tree::{Edge, NodeRole, SkinNode, TargetRule};

use super::target;
use super::target_tests::{button, laid, sheet, target_of};
use super::tests::{acts, fixed, n};

/// A 200 x 20 strip at the window's top, with one trigger.
fn strip(t: &NodeTrigger) -> SkinNode {
    acts(
        n("TopRow", NodeRole::Decor, fixed(0, 200), fixed(0, 20)),
        t.on,
        t.run.clone(),
    )
}

/// A 180 x 10 top grip over the strip.
fn grip() -> SkinNode {
    acts(
        n("ResizeTop", NodeRole::Button, fixed(10, 180), fixed(0, 10)),
        SkinEvent::LeftDown,
        SkinAction::Resize(Edge::Top),
    )
}

/// The served grip, and its [`target::Grip`] laid in the served tree.
fn served(l: &WindowLayout) -> (&SkinNode, target::Grip) {
    let tree = l.tree.as_ref().unwrap();
    let at = target::window(l);
    let g = target::grips(tree, at, &target::targets(tree, at));
    let g = g.into_iter().find(|g| g.laid.id == "ResizeTop").unwrap();
    (tree.root.iter().find(|n| n.id == "ResizeTop").unwrap(), g)
}

#[test]
fn a_grip_over_a_sys_menu_strip_keeps_its_band_and_serves_the_right_click() {
    for run in [
        SkinAction::ContextMenu(EngineMenu::Sys),
        SkinAction::Menu(EngineMenu::Sys),
    ] {
        let sys = NodeTrigger {
            on: SkinEvent::RightClick,
            run,
        };
        let l = laid(vec![strip(&sys), grip()]);
        let (node, g) = served(&l);
        assert_eq!(g.laid.area, [10, 0, 180, 24]);
        assert!(g.holes.is_empty(), "{:?}", g.holes);
        assert_eq!(g.laid.rule, TargetRule::Size);
        assert_eq!(g.takes, vec![sys.clone()]);
        assert_eq!(
            target_of(&l, "ResizeTop").map(|t| t.area),
            Some([0, 0, 180, 24])
        );
        // It still resizes on a left drag, and serves the strip's
        // right-click once.
        assert_eq!(node.resize_edge(), Some(Edge::Top));
        assert_eq!(node.triggers.iter().filter(|t| **t == sys).count(), 1);
        assert_eq!(
            target::grip_holes(l.tree.as_ref().unwrap(), target::window(&l)),
            Vec::new()
        );
    }
}

#[test]
fn a_strip_with_any_other_trigger_still_cuts_the_grip() {
    let tool = NodeTrigger {
        on: SkinEvent::RightClick,
        run: SkinAction::Menu(EngineMenu::Tool),
    };
    let l = laid(vec![strip(&tool), grip()]);
    let (node, g) = served(&l);
    assert_eq!(g.holes, vec![[10, 0, 180, 20]]);
    assert_eq!(g.laid.rule, TargetRule::Short);
    assert!(g.takes.is_empty());
    assert!(!node.triggers.contains(&tool));
}

#[test]
fn a_node_already_24_px_is_unchanged() {
    let l = laid(vec![button("Big", (50, 10), (24, 30))]);
    assert_eq!(target_of(&l, "Big"), None);
    let out = sheet(&l);
    assert!(out.contains("[data-skin-node=\"Big\"] {\n    position: absolute; left: 50px; width: 24px; top: 10px; height: 30px; z-index: 0;\n  }"), "{out}");
}
