//! The inset of a text field (plan SK5, the contract with the shell): a
//! pack may lay an interactive sibling over a text field's box. Symbiosis
//! stretches `UsernameContainer` (the name field) across its top and lays
//! `ButtonNotificationsTab`, the 26 x 24 notifications gear, over its end,
//! so a long name ran under the gear with no ellipsis to show it was cut.
//!
//! The Host serves the field `inset: {left, right}`, how far the siblings
//! that take input reach into it from each edge, in the tree laid at the
//! window's served minimum, and the shell lays the field's text and its
//! ellipsis between them; the sibling keeps its place and stays pressable.
//!
//! - A text field is a node of an editable text role: the name, status
//!   and room fields ([`is_text`]).
//! - A covering sibling is a node beside it in the same parent that takes
//!   input ([`SkinNode::interactive`]: a button, tab, menu title or status
//!   list) and is not itself a text field: two fields the pack overlaps
//!   (Alloy's 205 px name field under the start of its status field) keep
//!   their pack's places, the field's own text box is the pack's call.
//! - A resize grip (a pointer trigger that resizes, [`SkinNode::resize_edge`])
//!   draws nothing over the field and never covers it; nor does a node of
//!   another view (`shows_with` naming another tab: never shown with it).
//! - A sibling meeting the field's box (both axes, a positive area) covers
//!   it from the edge nearer the sibling's centre: from the right
//!   `field right - sibling left`, from the left `sibling right - field
//!   left`, at most the field's width. The largest reach on each side wins.
//! - A field no sibling meets serves no inset.
//!
//! Measured on the built-ins: Symbiosis's name field `[7, 82, 330, 13]`
//! under its gear `[309, 77, 26, 24]` serves `{left: 0, right: 28}` (the
//! resize strip `ResizeR` at x 332 reaches 5 px, under the gear's 28).

use rekindle_types::dto::skin::WindowLayout;
use rekindle_types::dto::skin_tree::{NodeInset, NodeRole, SkinNode};

use super::titles::{each_laid, place};

/// An editable text role (the name, status and room fields).
pub(super) const fn is_text(role: NodeRole) -> bool {
    matches!(
        role,
        NodeRole::NameField | NodeRole::StatusText | NodeRole::RoomName
    )
}

/// How far `by` reaches into `field` (both `[x, y, w, h]`) from each
/// side, `[left, right]`, or `None` when they do not meet.
fn reach(field: [i32; 4], by: [i32; 4]) -> Option<[i32; 2]> {
    let across = field[0].max(by[0]) < (field[0] + field[2]).min(by[0] + by[2]);
    let down = field[1].max(by[1]) < (field[1] + field[3]).min(by[1] + by[3]);
    if !(across && down) {
        return None;
    }
    let right_side = 2 * by[0] + by[2] > 2 * field[0] + field[2];
    let (left, right) = if right_side {
        (0, field[0] + field[2] - by[0])
    } else {
        (by[0] + by[2] - field[0], 0)
    };
    Some([left.min(field[2]), right.min(field[2])])
}

/// The inset of the text field at `field` among `siblings` laid at
/// `boxes`.
fn inset_of(field: usize, siblings: &[SkinNode], boxes: &[[i32; 4]]) -> Option<NodeInset> {
    let mut sides: Option<[i32; 2]> = None;
    for (i, s) in siblings.iter().enumerate() {
        let apart = s.shows_with.is_some()
            && siblings[field].shows_with.is_some()
            && s.shows_with != siblings[field].shows_with;
        if i == field || apart || is_text(s.role) || !s.interactive() || s.resize_edge().is_some() {
            continue;
        }
        if let Some([left, right]) = reach(boxes[field], boxes[i]) {
            let [l, r] = sides.unwrap_or([0, 0]);
            sides = Some([l.max(left), r.max(right)]);
        }
    }
    let px = |v: i32| u16::try_from(v.max(0)).unwrap_or(u16::MAX);
    sides.map(|[left, right]| NodeInset {
        left: px(left),
        right: px(right),
    })
}

fn walk(nodes: &mut [SkinNode], parent: [i32; 4]) {
    let boxes: Vec<[i32; 4]> = nodes.iter().map(|n| place(n, parent)).collect();
    let insets: Vec<Option<NodeInset>> = (0..nodes.len())
        .map(|i| {
            is_text(nodes[i].role)
                .then(|| inset_of(i, nodes, &boxes))
                .flatten()
        })
        .collect();
    for ((n, inset), b) in nodes.iter_mut().zip(insets).zip(boxes) {
        n.inset = inset;
        walk(&mut n.children, b);
    }
}

/// Sets the inset of every text field in `layout`'s tree and popups (see
/// the module doc), laid at the window's minimum size.
pub fn fields(layout: &mut WindowLayout) {
    each_laid(layout, walk);
}
