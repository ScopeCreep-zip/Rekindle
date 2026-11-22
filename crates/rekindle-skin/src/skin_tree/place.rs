//! Rule N4 (plan SK2-P1, design `sk2a/design.md` §3 "Rekindle's
//! placements"): the Host places the Communities view and the control that
//! opens it in every main-window tree, so no pack leaves Communities
//! unreachable. Applied by [`crate::manifest::parse`], the one path every
//! package (built-in, imported, library, draft) is read and served
//! through; a tree that already holds `View(Communities)` is left as is.
//!
//! - `View(Communities)` is a sibling view of `View(Friends)`, filling the
//!   views node as every view does; a views node with no views (a pack
//!   with one list) first gets its `View(Friends)`.
//! - The control, searched in the main tree and then in the popups it
//!   launches (StealthX's `MainMenuPopup` rail holds its tabs):
//!   - with tabs: a `Tab(Communities)` cloned from the last tab beside
//!     `Tab(Friends)` (its art and parts), one tab pitch further on;
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

/// A `Tab(Communities)` cloned from the last tab of `strip`.
fn clone_tab(strip: &mut Vec<SkinNode>, at: Target) {
    let tabs: Vec<usize> = (0..strip.len())
        .filter(|&i| matches!(strip[i].role, NodeRole::Tab(_)))
        .collect();
    let Some(&last) = tabs.last() else { return };
    let prev = tabs.len().checked_sub(2).map(|i| strip[tabs[i]].x);
    let mut tab = strip[last].clone();
    suffix_ids(&mut tab);
    tab.role = NodeRole::Tab(C);
    tab.shows_with = None;
    tab.x = step(tab.x, prev);
    retarget(&mut tab, at.group);
    tab.triggers.extend(at.press());
    strip.insert(last + 1, tab);
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
            .map(|t| t.on.clone())
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
    for root in &mut roots {
        // XGlow lays its tabs at the window's root (`Xglow.ini` `[Friends]`).
        if root.iter().any(|n| is_tab(n.role)) {
            clone_tab(root, at);
            return;
        }
        if let Some(strip) = find_mut(root.as_mut_slice(), &|n| holds(n, &is_tab)) {
            clone_tab(&mut strip.children, at);
            return;
        }
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
