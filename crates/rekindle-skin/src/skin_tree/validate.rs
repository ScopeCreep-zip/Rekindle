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
//! 4. two interactive nodes never overlap at the same z, unless they show
//!    with different views (a button row per tab);
//! 5. every interactive node has a box, and its 24×24 hit area (the
//!    stylesheet's, grown about its centre and shifted inward at the window's
//!    edges: lead decision 2, an edge grip's area stays in the window) lies
//!    inside the window; a text-sized node (a menu, tab or edit field, no
//!    art) is sized by its text at render, so its target is checked on the
//!    rendered window, not on its 0 px import box;
//! 6. each role appears at most once, except `Button`, `Decor`, `Resize`,
//!    `Tab`, and the role-less `Group`/`Part`;
//! 7. a `Views` node has a `View(Friends)`;
//! 8. every `ButtonRow(v)` has a matching `View(v)`;
//! 9. every art image is in the package;
//! 10. the frame tiles keep SK0's rules ([`crate::shape::check`] runs them
//!     on the same layout; the tree adds no frame tile). SK0's "frame tiles
//!     stay out of the content rect" belonged to the frame-around-our-app
//!     model and is withdrawn (lead decision 3): the window's background art
//!     may lie under any component, the views included; the views node draws
//!     opaque instead (its stylesheet rule's background).

use std::collections::{BTreeMap, BTreeSet};

use rekindle_types::dto::skin::{SkinError, SkinRule, WindowLayout};
use rekindle_types::dto::skin_tree::{NodeRole, NodeSpan, SkinNode, SkinTree, ViewKey};

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

/// A node sized by its text at render: a menu or tab drawn as text, or a
/// status or name edit field (a text box one line of its font tall: SimpleX's
/// `[StatusEdit]` has no art and no size), with no art (its import box is
/// 0 px on an axis).
#[must_use]
pub fn text_sized(n: &SkinNode) -> bool {
    let empty = |s: NodeSpan| matches!(s, NodeSpan::Fixed { size: 0, .. });
    n.art.is_none()
        && matches!(
            n.role,
            NodeRole::Menu(_) | NodeRole::Tab(_) | NodeRole::StatusText | NodeRole::NameField
        )
        && (empty(n.x) || empty(n.y))
}

/// The node's hit area in a `win_w × win_h` window: its box grown to 24×24
/// about its centre, then shifted inward to lie inside the window.
#[must_use]
pub fn hit_area(area: [i32; 4], (win_w, win_h): (i32, i32)) -> [i32; 4] {
    let grow = |at: i32, len: i32, win: i32| {
        let full = len.max(HIT);
        ((at - (full - len) / 2).min(win - full).max(0), full)
    };
    let (left, width) = grow(area[0], area[2], win_w);
    let (top, height) = grow(area[1], area[3], win_h);
    [left, top, width, height]
}

fn art_paths(n: &SkinNode) -> impl Iterator<Item = &String> {
    n.art
        .iter()
        .flat_map(|a| std::iter::once(&a.image).chain(a.states.values()))
}

/// Each node's view: the `shows_with` of it or its nearest ancestor. Two
/// rows of different views are never shown together (Classic's
/// `GameButton1`/`GameButton2` share a place in the Friends and Servers rows).
fn views_of<'a>(
    nodes: &'a [SkinNode],
    inherited: Option<ViewKey>,
    out: &mut BTreeMap<&'a str, Option<ViewKey>>,
) {
    for n in nodes {
        let v = n.shows_with.or(inherited);
        out.insert(n.id.as_str(), v);
        views_of(&n.children, v, out);
    }
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
    let out = layout.outsets;
    let w = i32::from(layout.min_width) + i32::from(out.left) + i32::from(out.right);
    let h = i32::from(layout.min_height) + i32::from(out.top) + i32::from(out.bottom);
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
        let sized = b[2] > 0 && b[3] > 0;
        if n.role.interactive() && !text_sized(n) && (!sized || !inside(hit_area(*b, (w, h)), w, h))
        {
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
    let mut shown = BTreeMap::new();
    views_of(&tree.root, None, &mut shown);
    let apart = |a: &str, b: &str| match (shown.get(a), shown.get(b)) {
        (Some(Some(x)), Some(Some(y))) => x != y,
        _ => false,
    };
    let live: Vec<_> = boxes.iter().filter(|(n, _)| n.role.interactive()).collect();
    for (i, (a, ab)) in live.iter().enumerate() {
        if live[i + 1..]
            .iter()
            .any(|(b, bb)| a.z == b.z && overlap(*ab, *bb) && !apart(&a.id, &b.id))
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
