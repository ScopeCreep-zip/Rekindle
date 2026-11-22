//! Rule N4 (plan SK2-P1, design `sk2a/design.md` §3 "Rekindle's
//! placements"): the Host places the Communities view and the control that
//! opens it in every main-window tree, so no pack leaves Communities
//! unreachable. Applied by [`crate::manifest::served`], the one path every
//! package (built-in, imported, library, draft) is validated and served
//! through, and never written into a package; a tree that already holds
//! `View(Communities)` is left as is.
//!
//! - `View(Communities)` is a sibling view of `View(Friends)`, filling the
//!   views node as every view does; a views node with no views (a pack
//!   with one list) first gets its `View(Friends)`.
//! - The control, searched in the main tree and then in the popups it
//!   launches (StealthX's `MainMenuPopup` rail holds its tabs):
//!   - with tabs: a `Tab(Communities)` cloned from the last tab beside
//!     `Tab(Friends)` (its art and parts), one tab pitch further on, down
//!     a rail whose tabs stack; when
//!     the strip stretches with the window and the clone runs past it
//!     (Symbiosis's six tabs fill its `TabsContainer`), the window's
//!     minimum width grows by the overrun, the pack's tabs and art as laid;
//!     a row spread over a fixed bar (Alloy's start/centre/end tabs) has
//!     no slot after its last tab, so its tabs pack edge to edge from the
//!     bar's start and the bar, and the window, widen to hold them;
//!   - with Combat's knob: a `Knob(Communities)` stop after the last
//!     stop, which now turns to it, and which turns on where the last
//!     stop turned;
//!   - with neither: a `Tab(Communities)` above the views node.
//! - The switch is a served trigger: `ShowView(Communities)`, and when the
//!   views node is a paged control group, its `GroupPage` to the new view.

use rekindle_types::dto::skin::WindowLayout;
use rekindle_types::dto::skin_action::{EngineView, NodeTrigger, SkinAction, SkinEvent};
use rekindle_types::dto::skin_tree::{Anchor, NodeRole, NodeSpan, SkinNode};
use rekindle_types::dto::skin_tree_role::ViewKey;

const C: ViewKey = ViewKey::Communities;

fn find_mut<'a>(
    nodes: &'a mut [SkinNode],
    f: &dyn Fn(&SkinNode) -> bool,
) -> Option<&'a mut SkinNode> {
    for n in nodes {
        if f(n) {
            return Some(n);
        }
        if let Some(hit) = find_mut(&mut n.children, f) {
            return Some(hit);
        }
    }
    None
}

fn fill_view(views: &SkinNode, v: ViewKey) -> SkinNode {
    let fill = NodeSpan::Stretch { start: 0, end: 0 };
    SkinNode {
        id: format!("{}.{v:?}", views.id),
        role: NodeRole::View(v),
        x: fill,
        y: fill,
        z: views.z,
        art: None,
        text: None,
        shows_with: Some(v),
        control_group: None,
        triggers: Vec::new(),
        children: Vec::new(),
    }
}

/// Where the switch goes: the views node's control group and the new
/// view's page in it.
#[derive(Clone, Copy)]
struct Target {
    group: Option<u16>,
    page: u16,
}

impl Target {
    fn actions(self) -> Vec<SkinAction> {
        let mut out = vec![SkinAction::ShowView(EngineView::Communities)];
        if let Some(group) = self.group {
            out.push(SkinAction::GroupPage {
                group,
                page: self.page,
            });
        }
        out
    }

    fn press(self) -> Vec<NodeTrigger> {
        self.actions()
            .into_iter()
            .map(|run| NodeTrigger {
                on: SkinEvent::LeftDown,
                run,
            })
            .collect()
    }
}

/// Places the view; `None` when the tree has no views node or already
/// holds the Communities view.
fn place_view(root: &mut [SkinNode]) -> Option<(Target, SkinNode)> {
    let views = find_mut(root, &|n| n.role == NodeRole::Views)?;
    if views.children.iter().any(|c| c.role == NodeRole::View(C)) {
        return None;
    }
    if !views
        .children
        .iter()
        .any(|c| matches!(c.role, NodeRole::View(_)))
    {
        let friends = fill_view(views, ViewKey::Friends);
        views.children.push(friends);
    }
    let view = fill_view(views, C);
    views.children.push(view);
    let page = u16::try_from(views.children.len() - 1).ok()?;
    let at = Target {
        group: views.control_group,
        page,
    };
    Some((at, views.clone()))
}

fn suffix_ids(n: &mut SkinNode) {
    n.id.push_str(".Communities");
    for c in &mut n.children {
        suffix_ids(c);
    }
}

/// The clone's triggers: no app signal (that was the old tab's: Symbiosis
/// shows Friends on `StartingLogin`), no switch to the old view, and its
/// shown-state art follows the Communities view.
fn retarget(n: &mut SkinNode, views_group: Option<u16>) {
    n.triggers.retain(|t| {
        !matches!(t.on, SkinEvent::App(_))
            && !matches!(t.run, SkinAction::ShowView(_))
            && !matches!(t.run, SkinAction::GroupPage { group, .. } if Some(group) == views_group)
    });
    for t in &mut n.triggers {
        if matches!(t.on, SkinEvent::Show(_)) {
            t.on = SkinEvent::Show(EngineView::Communities);
        }
    }
    for c in &mut n.children {
        retarget(c, views_group);
    }
}

fn step(last: NodeSpan, prev: Option<NodeSpan>) -> NodeSpan {
    let NodeSpan::Fixed { at, size } = last else {
        return last;
    };
    let pitch =
        |a: i16, p: Option<i16>| p.map_or_else(|| i16::try_from(size).unwrap_or(0), |p| a - p);
    let at = match (at, prev) {
        (
            Anchor::Start(a),
            Some(NodeSpan::Fixed {
                at: Anchor::Start(p),
                ..
            }),
        ) => Anchor::Start(a + pitch(a, Some(p))),
        (Anchor::Start(a), _) => Anchor::Start(a + pitch(a, None)),
        (
            Anchor::End(a),
            Some(NodeSpan::Fixed {
                at: Anchor::End(p), ..
            }),
        ) => Anchor::End(a + pitch(a, Some(p))),
        (Anchor::End(a), _) => Anchor::End(a - pitch(a, None)),
        (other, _) => other,
    };
    NodeSpan::Fixed { at, size }
}

/// A `Tab(Communities)` cloned from the last tab of `strip`; its id.
fn clone_tab(strip: &mut Vec<SkinNode>, at: Target) -> Option<String> {
    let tabs: Vec<usize> = (0..strip.len())
        .filter(|&i| matches!(strip[i].role, NodeRole::Tab(_)))
        .collect();
    let &last = tabs.last()?;
    let prev = tabs.len().checked_sub(2).map(|i| &strip[tabs[i]]);
    let mut tab = strip[last].clone();
    suffix_ids(&mut tab);
    tab.role = NodeRole::Tab(C);
    tab.shows_with = None;
    // A rail stacks its tabs (StealthX's `TabHeader`: one x, 21 px apart).
    match prev {
        Some(p) if p.x == tab.x && p.y != tab.y => tab.y = step(tab.y, Some(p.y)),
        _ => tab.x = step(tab.x, prev.map(|p| p.x)),
    }
    retarget(&mut tab, at.group);
    tab.triggers.extend(at.press());
    let id = tab.id.clone();
    strip.insert(last + 1, tab);
    Some(id)
}

/// A row spread over its strip: the last tab flush with the strip's end
/// and the one before it not (Alloy's `TabButtonBar`: start, centre, end),
/// so no slot follows the last tab.
fn spread(strip: &[SkinNode]) -> bool {
    let anchor = |n: &SkinNode| match n.x {
        NodeSpan::Fixed { at, .. } => Some(at),
        NodeSpan::Stretch { .. } => None,
    };
    let mut tabs = strip.iter().filter(|n| matches!(n.role, NodeRole::Tab(_)));
    match (tabs.next_back(), tabs.next_back()) {
        (Some(last), Some(prev)) => {
            matches!(anchor(last), Some(Anchor::End(_)))
                && !matches!(anchor(prev), Some(Anchor::End(_)))
        }
        _ => false,
    }
}

/// Lays a spread row's tabs edge to edge from the strip's start, each at
/// its own width, and a fixed strip as wide as the row.
fn pack_row(strip: &mut SkinNode) {
    let mut at: u16 = 0;
    for tab in strip
        .children
        .iter_mut()
        .filter(|n| matches!(n.role, NodeRole::Tab(_)))
    {
        if let NodeSpan::Fixed { size, .. } = tab.x {
            let start = i16::try_from(at).unwrap_or(i16::MAX);
            tab.x = NodeSpan::Fixed {
                at: Anchor::Start(start),
                size,
            };
            at = at.saturating_add(size);
        }
    }
    if let NodeSpan::Fixed { at: anchor, .. } = strip.x {
        strip.x = NodeSpan::Fixed {
            at: anchor,
            size: at,
        };
    }
}

/// Grows the window's minimum width by the run of the node `id` past its
/// strip on either side, when the strip stretches with the window (or is
/// the window), so the node sits in the strip at the minimum size. A fixed
/// strip is left as laid, and validation judges it (rule 3).
fn widen(layout: &mut WindowLayout, id: &str) {
    let Some(tree) = layout.tree.as_ref() else {
        return;
    };
    let out = layout.outsets;
    let w = i32::from(layout.min_width) + i32::from(out.left) + i32::from(out.right);
    let h = i32::from(layout.min_height) + i32::from(out.top) + i32::from(out.bottom);
    let boxes = tree.boxes(w, h);
    let Some(tab) = boxes.iter().find(|(n, _)| n.id == id).map(|b| b.1) else {
        return;
    };
    let strip = boxes
        .iter()
        .find(|(n, _)| n.children.iter().any(|c| c.id == id));
    let end = match strip {
        None => w,
        Some((n, b)) if matches!(n.x, NodeSpan::Stretch { .. }) => b[0] + b[2],
        Some(_) => return,
    };
    let start = strip.map_or(0, |(_, b)| b[0]);
    let run = (tab[0] + tab[2] - end).max(0) + (start - tab[0]).max(0);
    if let Ok(run) = u16::try_from(run) {
        layout.min_width = layout.min_width.saturating_add(run);
    }
}

/// A `Knob(Communities)` stop after the last stop of `knob`.
fn add_knob(knob: &mut Vec<SkinNode>, knob_group: Option<u16>, at: Target) {
    let Some(last) = knob
        .iter()
        .rposition(|c| matches!(c.role, NodeRole::Knob(_)))
    else {
        return;
    };
    let Ok(stop) = u16::try_from(knob.len()) else {
        return;
    };
    let mut new = knob[last].clone();
    suffix_ids(&mut new);
    new.role = NodeRole::Knob(C);
    new.shows_with = Some(C);
    let views_group = at.group;
    for t in &mut knob[last].triggers {
        match &mut t.run {
            SkinAction::GroupPage { group, page } if Some(*group) == knob_group => *page = stop,
            SkinAction::GroupPage { group, page } if Some(*group) == views_group => *page = at.page,
            SkinAction::ShowView(v) => *v = EngineView::Communities,
            _ => {}
        }
    }
    if views_group.is_none() {
        let presses: Vec<SkinEvent> = knob[last]
            .triggers
            .iter()
            .filter(|t| t.presses())
            .map(|t| t.on)
            .collect();
        for on in presses {
            knob[last].triggers.push(NodeTrigger {
                on,
                run: SkinAction::ShowView(EngineView::Communities),
            });
        }
    }
    knob.push(new);
}

/// A `Tab(Communities)` above `views`, for a pack with no tab or knob.
fn host_tab(views: &SkinNode, at: Target) -> SkinNode {
    const H: u16 = 16;
    let top = match views.y {
        NodeSpan::Stretch { start, .. } => start,
        NodeSpan::Fixed {
            at: Anchor::Start(s),
            ..
        } => s,
        NodeSpan::Fixed { .. } => 0,
    };
    let lift = i16::try_from(H).unwrap_or(0);
    SkinNode {
        id: format!("{}.CommunitiesTab", views.id),
        role: NodeRole::Tab(C),
        x: views.x,
        y: NodeSpan::Fixed {
            at: Anchor::Start((top - lift).max(0)),
            size: H,
        },
        z: views.z,
        art: None,
        text: None,
        shows_with: None,
        control_group: None,
        triggers: at.press(),
        children: Vec::new(),
    }
}

fn holds(n: &SkinNode, f: &dyn Fn(NodeRole) -> bool) -> bool {
    n.children.iter().any(|c| f(c.role))
}

/// Places the Communities view and its control in the main window's tree
/// and its popups (rule N4).
pub fn communities(layout: &mut WindowLayout) {
    let Some(tree) = layout.tree.as_mut() else {
        return;
    };
    let Some((at, views)) = place_view(&mut tree.root) else {
        return;
    };
    let is_tab = |r: NodeRole| r == NodeRole::Tab(ViewKey::Friends);
    let is_knob = |r: NodeRole| matches!(r, NodeRole::Knob(_));
    let roots =
        std::iter::once(&mut tree.root).chain(layout.popups.values_mut().map(|p| &mut p.tree.root));
    let mut roots: Vec<&mut Vec<SkinNode>> = roots.collect();
    for (i, root) in roots.iter_mut().enumerate() {
        // XGlow lays its tabs at the window's root (`Xglow.ini` `[Friends]`).
        let (id, packed) = if root.iter().any(|n| is_tab(n.role)) {
            (clone_tab(root, at), None)
        } else if let Some(strip) = find_mut(root.as_mut_slice(), &|n| holds(n, &is_tab)) {
            let packed = spread(&strip.children).then(|| strip.id.clone());
            let id = clone_tab(&mut strip.children, at);
            if packed.is_some() {
                pack_row(strip);
            }
            (id, packed)
        } else {
            continue;
        };
        // A popup's rail (StealthX) keeps its own size.
        if let (0, Some(id)) = (i, id) {
            for id in packed.iter().chain([&id]) {
                widen(layout, id);
            }
        }
        return;
    }
    for root in &mut roots {
        if let Some(knob) = find_mut(root.as_mut_slice(), &|n| holds(n, &is_knob)) {
            let group = knob.control_group;
            add_knob(&mut knob.children, group, at);
            return;
        }
    }
    let tab = host_tab(&views, at);
    if let Some(parent) = find_mut(roots[0].as_mut_slice(), &|n| {
        n.children.iter().any(|c| c.id == views.id)
    }) {
        parent.children.push(tab);
    } else {
        roots[0].push(tab);
    }
}
