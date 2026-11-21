//! The Host's checks on a component tree (plan SK2, design §5
//! "Validation"); a tree that breaks one refuses the package (no
//! fallback, no half-laid tree). The required set is what the pack
//! declares: nothing the original lacks is ever required.
//!
//! 1. every node has a role from the closed vocabulary (the importer's
//!    table refuses an unknown component), and a `Part` is never a root;
//! 2. ids are unique (the importer refuses an anchor that does not resolve);
//! 3. every node with a size lies inside the window (the frame grown by its
//!    outsets: the overhangs) at its smallest size;
//! 4. two interactive nodes never overlap at the same z;
//! 5. every interactive node has a box, and its 24×24 hit area (the
//!    stylesheet's) lies inside the window;
//! 6. each role appears at most once, except `Button`, `Decor`, `Resize`,
//!    `Tab`, and the role-less `Group`/`Part`;
//! 7. a `Views` node has a `View(Friends)`;
//! 8. every `ButtonRow(v)` has a matching `View(v)`;
//! 9. every art image is in the package;
//! 10. the frame tiles keep SK0's rules ([`crate::shape::check`] runs them
//!     on the same layout; the tree adds no frame tile).

use std::collections::{BTreeMap, BTreeSet};

use rekindle_types::dto::skin::{SkinError, SkinRule, WindowLayout};
use rekindle_types::dto::skin_tree::{NodeRole, SkinNode, SkinTree, ViewKey};

/// The most nodes a tree may hold.
pub const MAX_NODES: usize = 512;

/// The smallest hit area of an interactive node, px.
pub const HIT: i32 = 24;

fn bad(field: &str, expected: &str) -> SkinError {
    SkinError::new(
        field,
        SkinRule::BadValue {
            field: field.to_owned(),
            expected: expected.to_owned(),
        },
    )
}

fn overlap(a: [i32; 4], b: [i32; 4]) -> bool {
    a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3]
}

fn inside(b: [i32; 4], w: i32, h: i32) -> bool {
    b[0] >= 0 && b[1] >= 0 && b[0] + b[2] <= w && b[1] + b[3] <= h
}

/// The node's hit area: its box grown to 24×24 about its centre.
#[must_use]
pub fn hit_area(b: [i32; 4]) -> [i32; 4] {
    let grow = |at: i32, len: i32| {
        let l = len.max(HIT);
        (at - (l - len) / 2, l)
    };
    let (x, w) = grow(b[0], b[2]);
    let (y, h) = grow(b[1], b[3]);
    [x, y, w, h]
}

fn art_paths(n: &SkinNode) -> impl Iterator<Item = &String> {
    n.art
        .iter()
        .flat_map(|a| std::iter::once(&a.image).chain(a.states.values()))
}

/// Refuses `tree` (laid in `layout`) when it breaks a rule above.
///
/// # Errors
/// The first rule the tree breaks, its field under `field`.
pub fn check(
    field: &str,
    tree: &SkinTree,
    layout: &WindowLayout,
    images: &BTreeMap<String, Vec<u8>>,
) -> Result<(), SkinError> {
    let o = layout.outsets;
    let w = i32::from(layout.min_width) + i32::from(o.left) + i32::from(o.right);
    let h = i32::from(layout.min_height) + i32::from(o.top) + i32::from(o.bottom);
    let boxes = tree.boxes(w, h);
    if boxes.len() > MAX_NODES {
        return Err(bad(field, "at most 512 nodes"));
    }
    if tree.root.iter().any(|n| n.role == NodeRole::Part) {
        return Err(bad(field, "rule 1: a part inside its component"));
    }
    let (mut ids, mut roles) = (BTreeSet::new(), BTreeSet::new());
    for (n, b) in &boxes {
        let at = format!("{field}.{}", n.id);
        if !ids.insert(n.id.as_str()) {
            return Err(bad(&at, "rule 2: a unique id"));
        }
        if b[2] > 0 && b[3] > 0 && !inside(*b, w, h) {
            return Err(bad(&at, "rule 3: a node inside the window"));
        }
        if n.role.interactive() && (b[2] <= 0 || b[3] <= 0 || !inside(hit_area(*b), w, h)) {
            return Err(bad(
                &at,
                "rule 5: an interactive node with a 24x24 hit area in the window",
            ));
        }
        if !n.role.repeats() && !roles.insert(n.role) {
            return Err(bad(&at, "rule 6: one node per role"));
        }
        if art_paths(n).any(|p| !images.contains_key(p)) {
            return Err(bad(&at, "rule 9: art in the package"));
        }
    }
    let live: Vec<_> = boxes.iter().filter(|(n, _)| n.role.interactive()).collect();
    for (i, (a, ab)) in live.iter().enumerate() {
        if live[i + 1..]
            .iter()
            .any(|(b, bb)| a.z == b.z && overlap(*ab, *bb))
        {
            return Err(bad(
                &format!("{field}.{}", a.id),
                "rule 4: no interactive overlap at one z",
            ));
        }
    }
    if roles.contains(&NodeRole::Views) && !roles.contains(&NodeRole::View(ViewKey::Friends)) {
        return Err(bad(field, "rule 7: the views hold the friends view"));
    }
    for r in &roles {
        if let NodeRole::ButtonRow(v) = r {
            if !roles.contains(&NodeRole::View(*v)) {
                return Err(bad(field, "rule 8: a button row's view"));
            }
        }
    }
    Ok(())
}
