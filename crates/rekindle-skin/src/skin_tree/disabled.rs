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

/// The pack ships a disabled state image for the node's art or one of its
/// parts' (the importer's `--disabled` state variant), and the Host draws
/// no words over it (a Host label, such as the Communities tab's, is the
/// shell's to dim).
fn pack_art(n: &SkinNode) -> bool {
    let disabled = |node: &SkinNode| {
        node.art
            .as_ref()
            .is_some_and(|a| a.states.contains_key(&RegionState::Disabled))
    };
    n.label.is_none()
        && (disabled(n)
            || n.children
                .iter()
                .any(|c| c.role == NodeRole::Part && disabled(c)))
}
