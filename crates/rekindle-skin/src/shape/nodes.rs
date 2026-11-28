//! The component tree's art in the window's outline (plan SK2-S1r; SK0:
//! the art is the window's shape). A tree node draws its normal image at
//! its box's top left, at its own size, clipped to its box
//! (`skin_tree/css.rs`: `background: url(..) 0 0 no-repeat`; decision 15
//! already cropped a fixed tile's images to its parent at import), so
//! wherever that image is opaque the window must be too: Classic's header
//! band, StealthX's AdArea, Total Chaos's tab strip are node art, not
//! frame tiles.
//!
//! Only the art the window shows when it opens counts: the friends view
//! (a node of another view, or under another view, is hidden until its
//! tab is chosen) and the first child of each button bar, which shows one
//! child at a time (`skin_tree/validate.rs` `pages_of`).

use rekindle_types::dto::skin_tree::{NodeRole, SkinNode, SkinTree, ViewKey};

/// Whether `n` and its subtree are hidden when the window opens.
fn other_view(n: &SkinNode) -> bool {
    matches!(n.shows_with, Some(v) if v != ViewKey::Friends)
        || matches!(n.role, NodeRole::View(v) if v != ViewKey::Friends)
}

fn walk<'a>(nodes: &'a [SkinNode], out: &mut Vec<&'a str>) {
    for n in nodes {
        if other_view(n) {
            continue;
        }
        if n.art.is_some() {
            out.push(n.id.as_str());
        }
        if n.role == NodeRole::ButtonBar {
            walk(n.children.get(..1).unwrap_or_default(), out);
        } else {
            walk(&n.children, out);
        }
    }
}

/// The nodes whose normal art joins the outline, with their boxes in a
/// `width × height` window (logical px), in [`SkinTree::boxes`] order:
/// the order of their masks after the tile masks
/// ([`super::of_layout`]).
#[must_use]
pub fn outline_nodes(tree: &SkinTree, width: i32, height: i32) -> Vec<(&SkinNode, [i32; 4])> {
    let mut ids = Vec::new();
    walk(&tree.root, &mut ids);
    tree.boxes(width, height)
        .into_iter()
        .filter(|(n, _)| ids.contains(&n.id.as_str()))
        .collect()
}
