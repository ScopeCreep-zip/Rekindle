use rekindle_types::dto::skin::{SkinManifest, SkinWindow};
use rekindle_types::dto::skin_tree::{SkinNode, TargetRule};

use super::*;
use crate::skin_tree::target;

fn symbiosis() -> &'static [u8] {
    builtin::find(SOURCE).unwrap().1.bytes
}

fn alternate() -> SkinManifest {
    let bytes = symbiosis();
    served(bytes, &package::read(bytes).unwrap(), address())
        .unwrap()
        .0
}

fn original() -> SkinManifest {
    manifest::served(&package::read(symbiosis()).unwrap()).unwrap()
}

/// Every node of `m` in document order: its window, popup, id and triggers.
fn nodes(m: &SkinManifest) -> Vec<String> {
    fn walk(at: &str, nodes: &[SkinNode], out: &mut Vec<String>) {
        for n in nodes {
            out.push(format!("{at} {} {:?}", n.id, n.triggers));
            walk(at, &n.children, out);
        }
    }
    let mut out = Vec::new();
    for (w, l) in &m.windows {
        if let Some(t) = &l.tree {
            walk(&format!("{w:?}"), &t.root, &mut out);
        }
        for (p, pop) in &l.popups {
            walk(&format!("{w:?} {p}"), &pop.tree.root, &mut out);
        }
    }
    out
}

/// Plan SK14: the alternate serves no `Short` target in any window or
/// popup, where Symbiosis serves three: its `ResizeGripperTR` grip in its
/// buddy list, in its chat window and in the Community window, which wears
/// the chat window's frame (plan SK17-6, `goldens/targets-short.txt`; its
/// top-left and top grips take the `TopRow` system menu, `target::grips`).
/// Plan SK15's settle pass grows its other short boxes in both windows
/// (`ButtonMaximize`, `ButtonClose`, and the buddy list's `StatusChanger`,
/// each cut to 21 or 22 px by a neighbour) to 24 x 24 into the free room on
/// their far side, so they serve `Size`.
#[test]
fn the_alternate_serves_no_short_target() {
    assert_eq!(short_targets(&original()), 3);
    assert_eq!(short_targets(&alternate()), 0);
    assert_eq!(builtin_short_targets(SOURCE).unwrap(), Some(3));
    assert_eq!(builtin_short_targets(ID).unwrap(), Some(0));
    assert_eq!(builtin_short_targets("builtin:nope").unwrap(), None);
}

/// The four moved controls, laid at the served minimum: each 24 x 24.
#[test]
fn the_moved_controls_serve_24_px_targets() {
    let m = alternate();
    let layout = &m.windows[&SkinWindow::Main];
    let tree = layout.tree.as_ref().unwrap();
    let laid = target::targets(tree, target::window(layout));
    let area = |id: &str| {
        let l = laid.iter().find(|l| l.id == id).unwrap();
        (l.area, l.rule)
    };
    assert_eq!(area("ButtonMinimize"), ([259, 4, 24, 24], TargetRule::Size));
    assert_eq!(area("ButtonMaximize"), ([283, 4, 24, 24], TargetRule::Size));
    assert_eq!(area("ButtonClose"), ([307, 4, 24, 24], TargetRule::Size));
    assert_eq!(
        area("StatusChanger"),
        ([329, 102, 24, 24], TargetRule::Size)
    );
    let grips = target::grip_targets(tree, target::window(layout), &laid);
    let tr = grips.iter().find(|g| g.id == "ResizeGripperTR").unwrap();
    assert_eq!((tr.area, tr.rule), ([331, 0, 24, 24], TargetRule::Size));
}

/// No button lost: the alternate serves exactly Symbiosis's nodes, in its
/// order, with its triggers; only its name differs.
#[test]
fn the_alternate_keeps_every_node_and_trigger() {
    let (a, o) = (alternate(), original());
    assert_eq!(nodes(&a), nodes(&o));
    assert_eq!(a.name, NAME);
    assert_eq!(a.id, o.id);
}

/// Its own address, derived from Symbiosis's; only Symbiosis's bytes are
/// served there, and any other address serves the pack as laid.
#[test]
fn the_alternate_has_its_own_address() {
    let sym = blake3::hash(symbiosis()).to_hex().to_string();
    assert!(crate::serve::is_hash(address()));
    assert_ne!(address(), sym);
    assert!(builtin::by_address(address()).is_none());
    let p = package::read(symbiosis()).unwrap();
    assert_eq!(served(symbiosis(), &p, &sym).unwrap().0, original());
    let other = builtin::find("builtin:combat").unwrap().1.bytes;
    let q = package::read(other).unwrap();
    assert!(served(other, &q, address()).is_err());
    assert!(is_builtin(ID) && is_builtin(SOURCE) && !is_builtin("builtin:nope"));
    assert_eq!(pack_id(ID), SOURCE);
    assert_eq!(pack_id("builtin:combat"), "builtin:combat");
}
