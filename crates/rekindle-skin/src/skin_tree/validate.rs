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
//! 4. interactive nodes may overlap (plan SK2 lead decision 6): stacking is
//!    the pack's document order (a higher `Z` above, then a later sibling
//!    above, a child above its parent), as Xfire drew; an interactive node
//!    is refused only when one later-stacked interactive node covers its
//!    whole box (it could never be reached), unless the two show with
//!    different views (a button row per tab) or lie on different pages of
//!    one button bar (a `Class=TabControl` shows one child at a time:
//!    StealthX's `ChatTab` pages `HostChatButton`/`JoinChatButton`/
//!    `LeaveChatButton` by chat state, `GroupMessage(0/1/2)`,
//!    `components.xml:705-712`); an edge grip yields to the
//!    interactive nodes above it in the window's global order (lead
//!    decision 10: effective z through the parent chain, [`Stack::yields`]):
//!    it stacks below its siblings at its `Z` ([`stack_z`]), its own box
//!    takes no pointer events and its hit area leaves their boxes
//!    ([`grip_area`]);
//! 5. every interactive node has a box, and its 24×24 hit area (the
//!    stylesheet's, grown about its centre and shifted inward at the window's
//!    edges: lead decision 2, an edge grip's area stays in the window) lies
//!    inside the window; a text-sized node (a menu, tab or edit field, no
//!    art) is sized by its text at render, so its target is checked on the
//!    rendered window, not on its 0 px import box;
//! 6. each singleton role appears at most once (lead decision 7: the title
//!    strip, the window boxes, the name and status fields, the views;
//!    [`NodeRole::singleton`]); the rest repeat per view or tab;
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
use rekindle_types::dto::skin_tree::{NodeArt, NodeRole, NodeSpan, SkinNode, SkinTree, ViewKey};

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

/// Whether the node is seen or touched (plan SK2 lead decision 14): it
/// draws art, takes input, or is a part the shell renders. An art-less
/// wrapper (`Group`) is a layout box only (Classic's `StatusWrapper`, whose
/// slices the status strip draws): it never grows the window, and what of
/// it lies outside is never seen.
#[must_use]
pub fn shown(n: &SkinNode) -> bool {
    n.art.is_some() || n.role != NodeRole::Group
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

/// An edge grip's hit area (lead decisions 6 and 10): its own box less,
/// along its long axis, each node in `live` (the interactive nodes the grip
/// yields to, [`Stack::yields`]) that lies over it (the larger piece kept:
/// xSLIMFIRE's `ResizeUp` stops short of the `CloseBox` over its strip),
/// then [`hit_area`], trimmed back toward that box along each axis the grip
/// is thinner than 24 px so it never lies over a node in `live`. Empty
/// when the nodes it yields to cover the whole strip.
#[must_use]
pub fn grip_area(area: [i32; 4], window: (i32, i32), live: &[[i32; 4]]) -> [i32; 4] {
    let mut own = area;
    for other in live {
        if overlap(own, *other) {
            let axis = usize::from(own[2] < own[3]);
            let (lo, hi) = (own[axis], own[axis] + own[axis + 2]);
            let before = other[axis].clamp(lo, hi) - lo;
            let after = hi - (other[axis] + other[axis + 2]).clamp(lo, hi);
            if before >= after {
                own[axis + 2] = before;
            } else {
                own[axis] = hi - after;
                own[axis + 2] = after;
            }
        }
    }
    if own[2] <= 0 || own[3] <= 0 {
        return [own[0], own[1], 0, 0];
    }
    let mut hit = hit_area(own, window);
    for other in live {
        for axis in [0, 1] {
            if own[axis + 2] >= HIT || !overlap(hit, *other) {
                continue;
            }
            let (lo, hi) = (hit[axis], hit[axis] + hit[axis + 2]);
            let (own_lo, own_hi) = (own[axis], own[axis] + own[axis + 2]);
            let (o_lo, o_hi) = (other[axis], other[axis] + other[axis + 2]);
            // Keep the side of the grip's own box away from the node.
            let (new_lo, new_hi) = if o_lo >= own_hi {
                (lo, o_lo.max(own_hi))
            } else if o_hi <= own_lo {
                (o_hi.min(own_lo), hi)
            } else {
                (own_lo, own_hi)
            };
            hit[axis] = new_lo;
            hit[axis + 2] = new_hi - new_lo;
        }
    }
    hit
}

/// A node's z-index in the stylesheet (lead decision 6): twice its `Z`, less
/// one for an edge grip, so a grip stacks below its siblings at its `Z`
/// (its hit area yields to the nodes it overlaps) and above those below it.
#[must_use]
pub fn stack_z(n: &SkinNode) -> i32 {
    2 * i32::from(n.z) - i32::from(matches!(n.role, NodeRole::Resize(_)))
}

/// The window's global stacking order (lead decisions 6 and 10).
pub struct Stack<'a> {
    /// Each node's path of its and its ancestors' `(`[`stack_z`]`, sibling
    /// index)`, root first: a path stacks above another when it is greater
    /// at their first difference, or extends it (a child above its parent).
    path: BTreeMap<&'a str, Vec<(i32, usize)>>,
    /// Each node's effective z: the highest [`stack_z`] through its parent
    /// chain (xSLIMFIRE's `CloseBox` `Z=3` inside `MainTitleBar` `Z=1` is at
    /// 3, above the root `ResizeUp` `Z=2`).
    effective: BTreeMap<&'a str, i32>,
}

impl<'a> Stack<'a> {
    /// The stacking of every node under `root`.
    #[must_use]
    pub fn new(root: &'a [SkinNode]) -> Self {
        fn walk<'a>(nodes: &'a [SkinNode], path: &[(i32, usize)], s: &mut Stack<'a>) {
            for (i, n) in nodes.iter().enumerate() {
                let mut own = path.to_vec();
                own.push((stack_z(n), i));
                let z = own.iter().map(|(z, _)| *z).max().unwrap_or(0);
                walk(&n.children, &own, s);
                s.path.insert(n.id.as_str(), own);
                s.effective.insert(n.id.as_str(), z);
            }
        }
        let mut s = Stack {
            path: BTreeMap::new(),
            effective: BTreeMap::new(),
        };
        walk(root, &[], &mut s);
        s
    }

    /// Whether `a` stacks above `b` in document order.
    #[must_use]
    pub fn above(&self, a: &str, b: &str) -> bool {
        self.path.get(a) > self.path.get(b)
    }

    /// Whether the grip `grip` yields to `other` (lead decision 10): `other`
    /// is above it in the window's global order, by effective z, then by
    /// document order.
    #[must_use]
    pub fn yields(&self, grip: &str, other: &str) -> bool {
        match self.effective.get(other).cmp(&self.effective.get(grip)) {
            std::cmp::Ordering::Greater => true,
            std::cmp::Ordering::Less => false,
            std::cmp::Ordering::Equal => self.above(other, grip),
        }
    }
}

fn contains(outer: [i32; 4], inner: [i32; 4]) -> bool {
    outer[0] <= inner[0]
        && outer[1] <= inner[1]
        && inner[0] + inner[2] <= outer[0] + outer[2]
        && inner[1] + inner[3] <= outer[1] + outer[3]
}

fn art_paths(n: &SkinNode) -> impl Iterator<Item = &String> {
    n.art.iter().flat_map(NodeArt::images)
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

/// Each node's pages: for every button bar above it, the bar's id and the
/// index of the bar's child it lies under. A button bar is a tab control
/// (the importer makes one only from `Class=TabControl`/`TabbedControl`
/// or from INI slots grouped per view) and shows one child at a time.
fn pages_of<'a>(
    nodes: &'a [SkinNode],
    inherited: &[(&'a str, usize)],
    out: &mut BTreeMap<&'a str, Vec<(&'a str, usize)>>,
) {
    for n in nodes {
        out.insert(n.id.as_str(), inherited.to_vec());
        let mut below = inherited.to_vec();
        if n.role == NodeRole::ButtonBar {
            for (i, c) in n.children.iter().enumerate() {
                below.push((n.id.as_str(), i));
                pages_of(std::slice::from_ref(c), &below, out);
                below.pop();
            }
        } else {
            pages_of(&n.children, &below, out);
        }
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
    let (mut ids, mut singles, mut roles) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
    for (n, b) in &boxes {
        let at = format!("{field}.{}", n.id);
        if !ids.insert(n.id.as_str()) {
            return Err(bad(&at, "rule 2: a unique id"));
        }
        if shown(n) && b[2] > 0 && b[3] > 0 && !inside(*b, w, h) {
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
        if n.role.singleton() && !singles.insert(n.role) {
            return Err(bad(&at, "rule 6: one node per singleton role"));
        }
        roles.insert(n.role);
        if art_paths(n).any(|p| !images.contains_key(p)) {
            return Err(bad(&at, "rule 9: art in the package"));
        }
    }
    let mut shown = BTreeMap::new();
    views_of(&tree.root, None, &mut shown);
    let mut pages = BTreeMap::new();
    pages_of(&tree.root, &[], &mut pages);
    let other_page = |a: &str, b: &str| {
        let (Some(pa), Some(pb)) = (pages.get(a), pages.get(b)) else {
            return false;
        };
        pa.iter()
            .any(|(bar, i)| pb.iter().any(|(bar2, j)| bar == bar2 && i != j))
    };
    let apart = |a: &str, b: &str| {
        other_page(a, b)
            || match (shown.get(a), shown.get(b)) {
                (Some(Some(x)), Some(Some(y))) => x != y,
                _ => false,
            }
    };
    let stack = Stack::new(&tree.root);
    // A grip covers a node only when it does not yield to it (lead
    // decision 10): its hit area then leaves the node's box.
    let covers = |b: &SkinNode, a: &str| {
        stack.above(&b.id, a) && !(matches!(b.role, NodeRole::Resize(_)) && stack.yields(&b.id, a))
    };
    let live: Vec<_> = boxes
        .iter()
        .filter(|(n, b)| n.role.interactive() && b[2] > 0 && b[3] > 0)
        .collect();
    for (a, ab) in &live {
        if live.iter().any(|(b, bb)| {
            b.id != a.id && covers(b, &a.id) && contains(*bb, *ab) && !apart(&a.id, &b.id)
        }) {
            return Err(bad(
                &format!("{field}.{}", a.id),
                "rule 4: an interactive node another never wholly covers",
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
