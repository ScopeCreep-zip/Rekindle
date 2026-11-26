//! Who draws each interactive node's disabled look (plan E7.7 CR1, owner
//! ruling: a disabled button is visibly disabled and shows the Host's
//! reason): a serve pass ([`crate::manifest::place`]), never written into
//! a package.

use rekindle_types::dto::skin::{RegionState, WindowLayout};
use rekindle_types::dto::skin_tree::{DisabledBy, NodeRole, SkinNode};

/// Sets `disabled_by` on every interactive node of the window's tree and
/// its popups' trees, and clears it on every other node.
pub fn nodes(layout: &mut WindowLayout) {
    if let Some(tree) = layout.tree.as_mut() {
        mark(&mut tree.root);
    }
    for popup in layout.popups.values_mut() {
        mark(&mut popup.tree.root);
    }
}

fn mark(nodes: &mut [SkinNode]) {
    for n in nodes {
        n.disabled_by = n.interactive().then(|| {
            if pack_art(n) {
                DisabledBy::Pack
            } else {
                DisabledBy::Shell
            }
        });
        mark(&mut n.children);
    }
}

/// The pack ships a disabled look of its own for the node's art or one of
/// its parts' (the importer's `--disabled` state variant whose layers differ
/// from the normal ones: asset names are content hashes, so equal layer sets
/// draw the same pixels, as Combat's round Open/Leave buttons do), and the
/// Host draws no words over it (a Host label, such as the Communities tab's,
/// is the shell's to dim). A Disabled set equal to Normal would leave a
/// refused button looking pressable, so the shell dims it (owner rule: never
/// hidden, visibly disabled).
pub(super) fn pack_art(n: &SkinNode) -> bool {
    let distinct = |node: &SkinNode| {
        node.art.as_ref().is_some_and(|a| {
            a.states
                .get(&RegionState::Disabled)
                .is_some_and(|d| *d != a.normal)
        })
    };
    n.label.is_none()
        && (distinct(n)
            || n.children
                .iter()
                .any(|c| c.role == NodeRole::Part && distinct(c)))
}
