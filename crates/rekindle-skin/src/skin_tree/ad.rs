//! The ad's own box (plan E7.7; CLAUDE.md "Key Rules": the Rekindle
//! banner takes the place of Xfire's served ad). The shell draws its
//! banner over the node with role `AdArea`, so that role belongs to the
//! pack's ad, the `Class="AdView"` tile, never the frame around it:
//! StealthX's 234x60 `AdView` (`components.xml:638`) is centred in a
//! full-width `AdArea` (`Class="Tile"`, `components.xml:637`), Total
//! Chaos's 236x60 `AdArea` (`Class="AdView"`, `components.xml:359-363`)
//! in its 470 px `AdBar` art. The importer names both tiles' role from
//! their ad names and makes the inner one the frame's part
//! (`roles::role`: the same role inside itself), so the ad is that part.

use rekindle_types::dto::skin::WindowLayout;
use rekindle_types::dto::skin_tree::{NodeRole, SkinNode};

/// Moves the ad role from each frame in `layout`'s tree onto the ad it
/// holds; the frame keeps its art as decor, or is a plain group. A frame
/// with no ad inside stays the ad.
pub fn serve(layout: &mut WindowLayout) {
    if let Some(tree) = layout.tree.as_mut() {
        own_box(&mut tree.root);
    }
}

fn own_box(nodes: &mut [SkinNode]) {
    for n in nodes {
        if n.role != NodeRole::AdArea {
            own_box(&mut n.children);
            continue;
        }
        let framed = matches!(n.children.as_slice(), [ad] if ad.role == NodeRole::Part);
        if framed {
            n.children[0].role = NodeRole::AdArea;
            n.role = if n.art.is_some() {
                NodeRole::Decor
            } else {
                NodeRole::Group
            };
        }
    }
}
