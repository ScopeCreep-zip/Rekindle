//! The Communities view's button row (plan E7.7): in Xfire each buddy-list
//! view has its own row in the `ButtonBar` (Symbiosis's
//! `TabsButtonsContainer`, one `ButtonRow(<view>)` per view), and rule N4
//! ([`super::place`]) composes the Communities tab and view but no pack
//! has a row for it. The Host composes `ButtonRow(Communities)` the same
//! way, at serve time and never into a package, from the pack's own
//! vocabulary (owner ruling), each button by its meaning (lead ruling
//! 2026-11-24):
//!
//! - **Frame:** the pack's ChatRooms row, else its Friends row, cloned with
//!   its spacing, frame and separators; its first three button slots hold
//!   Open, Leave and New, and its other buttons and room-name field are
//!   dropped.
//! - **Art per slot**, the first the pack has:
//!
//!   | Button | Action | Art, in order |
//!   |---|---|---|
//!   | Open | [`SkinAction::OpenCommunity`] | `ChatRoomEnter`, the pack's Chat button (the first `ChatFriend` press: `ChatButton`, `FriendChatButton`, `ImButton`), `JoinButton@Servers` |
//!   | Leave | [`SkinAction::LeaveCommunity`] | `ChatRoomDelete`, `DeleteFiles`, `StopButton` |
//!   | New | [`SkinAction::StartCommunity`] | `ChatRoomCreate`, `AddServerButton`, the pack's Friends Add button (the first `AddFriend` press) |
//!
//!   With none, the slot is a plain button: no art, a text part with the
//!   Host's word.
//!   A word the pack drew on a button reads ours ([`super::titles`]).
//! - Open and Leave are never hidden: the Host's `cannot` for their
//!   capability (no community selected) disables them, its `can` enables
//!   them; pressed while refused, the Host answers "Select a community
//!   first.".

use rekindle_types::dto::skin::WindowLayout;
use rekindle_types::dto::skin_action::{
    AppSignal, Capability, EngineView, NodeState, NodeTrigger, SkinAction, SkinEvent,
};
use rekindle_types::dto::skin_tree::{NodeRole, NodeSpan, SkinNode};
use rekindle_types::dto::skin_tree_role::ViewKey;

const C: ViewKey = ViewKey::Communities;

/// The rows a frame is cloned from, in order.
const FRAMES: [(ViewKey, EngineView); 2] = [
    (ViewKey::ChatRooms, EngineView::ChatRooms),
    (ViewKey::Friends, EngineView::Friends),
];

/// Where a slot's art comes from.
enum Donor {
    Id(&'static str),
    /// The first button whose press runs this action.
    Press(SkinAction),
}

/// One button of the row.
struct Slot {
    action: SkinAction,
    capability: Option<Capability>,
    words: &'static str,
    donors: [Donor; 3],
}

const SLOTS: [Slot; 3] = [
    Slot {
        action: SkinAction::OpenCommunity,
        capability: Some(Capability::OpenCommunity),
        words: "Open",
        donors: [
            Donor::Id("ChatRoomEnter"),
            Donor::Press(SkinAction::ChatFriend),
            Donor::Id("JoinButton@Servers"),
        ],
    },
    Slot {
        action: SkinAction::LeaveCommunity,
        capability: Some(Capability::LeaveCommunity),
        words: "Leave",
        donors: [
            Donor::Id("ChatRoomDelete"),
            Donor::Id("DeleteFiles"),
            Donor::Id("StopButton"),
        ],
    },
    Slot {
        action: SkinAction::StartCommunity,
        capability: None,
        words: "New",
        donors: [
            Donor::Id("ChatRoomCreate"),
            Donor::Id("AddServerButton"),
            Donor::Press(SkinAction::AddFriend),
        ],
    },
];

fn buttons(n: &SkinNode) -> usize {
    n.children
        .iter()
        .map(|c| usize::from(c.role == NodeRole::Button) + buttons(c))
        .sum()
}

/// The first node under `nodes`, depth first, that `f` holds for.
pub(super) fn find<'a>(
    nodes: &'a [SkinNode],
    f: &dyn Fn(&SkinNode) -> bool,
) -> Option<&'a SkinNode> {
    nodes
        .iter()
        .find_map(|n| if f(n) { Some(n) } else { find(&n.children, f) })
}

fn donor_matches(n: &SkinNode, d: &Donor) -> bool {
    n.role == NodeRole::Button
        && match d {
            Donor::Id(id) => n.id == *id,
            Donor::Press(a) => n.triggers.iter().any(|t| t.presses() && t.run == *a),
        }
}

/// Each slot's donor button, searched in every tree of the window.
fn donors(roots: &[&[SkinNode]]) -> Vec<Option<SkinNode>> {
    SLOTS
        .iter()
        .map(|s| {
            s.donors.iter().find_map(|d| {
                roots
                    .iter()
                    .find_map(|r| find(r, &|n| donor_matches(n, d)))
                    .cloned()
            })
        })
        .collect()
}

fn holds_row(children: &[SkinNode]) -> bool {
    children
        .iter()
        .any(|c| matches!(c.role, NodeRole::ButtonRow(k) if FRAMES.iter().any(|s| s.0 == k)))
}

/// The children of the button bar: `nodes` itself when it holds the rows
/// (Combat lays its rows at the window's root), else a node's below.
fn find_bar(nodes: &mut Vec<SkinNode>) -> Option<&mut Vec<SkinNode>> {
    if holds_row(nodes) {
        return Some(nodes);
    }
    nodes.iter_mut().find_map(|n| find_bar(&mut n.children))
}

/// Drops every button after the first `keep` (counted in `seen`), and any
/// room-name field, with a wrapper left holding no button.
fn prune(nodes: &mut Vec<SkinNode>, keep: usize, seen: &mut usize) {
    nodes.retain_mut(|n| {
        if n.role == NodeRole::RoomName {
            return false;
        }
        if n.role == NodeRole::Button {
            *seen += 1;
            return *seen <= keep;
        }
        let had = buttons(n) > 0 || find(&n.children, &|c| c.role == NodeRole::RoomName).is_some();
        prune(&mut n.children, keep, seen);
        !had || buttons(n) > 0
    });
}

/// A run that only changes the button's look or sound, kept on a press.
fn cosmetic(run: &SkinAction) -> bool {
    matches!(
        run,
        SkinAction::SetState(_)
            | SkinAction::PlaySound(_)
            | SkinAction::Animate(_)
            | SkinAction::AnimateStop
            | SkinAction::ShowTile(_)
            | SkinAction::HideTile(_)
    )
}

/// The frame's view, which the clone's shown state moves to Communities.
#[derive(Clone, Copy)]
struct Src(ViewKey, EngineView);

/// `n` and its subtree: ids suffixed, app signals and view switches
/// dropped, feature presses dropped (the first one's event kept in
/// `press`), and the shown state following the Communities view.
fn retarget(n: &mut SkinNode, from: Src, press: &mut Option<SkinEvent>) {
    n.id.push_str(".Communities");
    if n.shows_with == Some(from.0) {
        n.shows_with = Some(C);
    }
    n.triggers.retain(|t| {
        if matches!(t.on, SkinEvent::App(_))
            || matches!(
                t.run,
                SkinAction::ShowView(_) | SkinAction::GroupPage { .. }
            )
        {
            return false;
        }
        if t.presses() && !cosmetic(&t.run) {
            press.get_or_insert(t.on);
            return false;
        }
        true
    });
    for t in &mut n.triggers {
        if t.on == SkinEvent::Show(from.1) {
            t.on = SkinEvent::Show(EngineView::Communities);
        }
    }
    for c in &mut n.children {
        retarget(c, from, press);
    }
}

/// The pack's own word on a button (`FindButtonText` "Find") reads `words`.
fn reword(n: &mut SkinNode, words: &str) {
    if let Some(l) = n.label.as_mut() {
        words.clone_into(&mut l.text);
    }
    for c in &mut n.children {
        reword(c, words);
    }
}

/// `span` at `size`'s size, its anchor kept; a stretching span stays.
fn resized(span: &NodeSpan, size: &NodeSpan) -> NodeSpan {
    match (span, size) {
        (NodeSpan::Fixed { at, .. }, NodeSpan::Fixed { size, .. }) => NodeSpan::Fixed {
            at: at.clone(),
            size: *size,
        },
        _ => span.clone(),
    }
}

/// Makes slot `n` the row's `slot`: the donor's art and parts at the
/// slot's place, else a plain button with the Host's word.
fn fill(n: &mut SkinNode, slot: &Slot, donor: Option<&SkinNode>, from: Src) {
    if let Some(d) = donor {
        n.art.clone_from(&d.art);
        n.text.clone_from(&d.text);
        n.label.clone_from(&d.label);
        n.children.clone_from(&d.children);
        n.x = resized(&n.x, &d.x);
        n.y = resized(&n.y, &d.y);
    } else {
        // The word-part path (StealthX's `FindButtonText`): an art-less
        // text part filling the button, which [`super::titles`] gives the
        // Host's word for the button's press.
        n.art = None;
        n.label = None;
        let fill = NodeSpan::Stretch { start: 0, end: 0 };
        n.children = vec![SkinNode {
            id: format!("Communities{}Text", slot.words),
            role: NodeRole::Part,
            x: fill.clone(),
            y: fill,
            z: n.z,
            art: None,
            text: None,
            label: None,
            inset: None,
            target: None,
            fit: None,
            shows_with: None,
            control_group: None,
            triggers: Vec::new(),
            children: Vec::new(),
        }];
    }
    let mut press = None;
    retarget(n, from, &mut press);
    n.id = format!("Communities{}", slot.words);
    n.triggers.push(NodeTrigger {
        on: press.unwrap_or(SkinEvent::Click),
        run: slot.action.clone(),
    });
    if let Some(cap) = slot.capability {
        n.triggers.push(NodeTrigger {
            on: SkinEvent::App(AppSignal::Cannot(cap)),
            run: SkinAction::SetState(NodeState::Disabled),
        });
        n.triggers.push(NodeTrigger {
            on: SkinEvent::App(AppSignal::Can(cap)),
            run: SkinAction::SetState(NodeState::Normal),
        });
    }
    reword(n, slot.words);
}

/// Fills the clone's button slots in document order (`i` counts them).
fn wire(nodes: &mut [SkinNode], donors: &[Option<SkinNode>], from: Src, i: &mut usize) {
    for n in nodes {
        if n.role != NodeRole::Button {
            wire(&mut n.children, donors, from, i);
            continue;
        }
        let Some(slot) = SLOTS.get(*i) else {
            return;
        };
        fill(n, slot, donors[*i].as_ref(), from);
        *i += 1;
    }
}

/// The row's own triggers and every non-button node's, retargeted.
fn retarget_frame(n: &mut SkinNode, from: Src) {
    let mut ignored = None;
    n.id.push_str(".Communities");
    if n.shows_with == Some(from.0) {
        n.shows_with = Some(C);
    }
    n.triggers.retain(|t| {
        !matches!(t.on, SkinEvent::App(_))
            && !matches!(
                t.run,
                SkinAction::ShowView(_) | SkinAction::GroupPage { .. }
            )
    });
    for t in &mut n.triggers {
        if t.on == SkinEvent::Show(from.1) {
            t.on = SkinEvent::Show(EngineView::Communities);
        }
    }
    for c in &mut n.children {
        if c.role == NodeRole::Button {
            continue;
        }
        if buttons(c) == 0 {
            retarget(c, from, &mut ignored);
        } else {
            retarget_frame(c, from);
        }
    }
}

/// Composes `ButtonRow(Communities)` in the main window's tree, else in a
/// popup it launches; a tree that has one, or has no frame row, is left.
pub fn row(layout: &mut WindowLayout) {
    let donors = {
        let mut roots: Vec<&[SkinNode]> = layout.tree.iter().map(|t| t.root.as_slice()).collect();
        roots.extend(layout.popups.values().map(|p| p.tree.root.as_slice()));
        donors(&roots)
    };
    let main = layout.tree.as_mut().map(|t| &mut t.root);
    let roots = main
        .into_iter()
        .chain(layout.popups.values_mut().map(|p| &mut p.tree.root));
    for root in roots {
        let Some(bar) = find_bar(root) else {
            continue;
        };
        if bar.iter().any(|c| c.role == NodeRole::ButtonRow(C)) {
            return;
        }
        let Some((at, from)) = FRAMES.iter().find_map(|&(k, v)| {
            bar.iter()
                .position(|c| c.role == NodeRole::ButtonRow(k))
                .map(|i| (i, Src(k, v)))
        }) else {
            return;
        };
        let mut clone = bar[at].clone();
        prune(&mut clone.children, SLOTS.len(), &mut 0);
        clone.role = NodeRole::ButtonRow(C);
        retarget_frame(&mut clone, from);
        wire(&mut clone.children, &donors, from, &mut 0);
        let last = bar
            .iter()
            .rposition(|c| matches!(c.role, NodeRole::ButtonRow(_)))
            .unwrap_or(at);
        bar.insert(last + 1, clone);
        return;
    }
}
