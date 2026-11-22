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
use rekindle_types::dto::skin_tree::{
    Anchor, NodeGlyph, NodeLabel, NodeRole, NodeSpan, PieceSlot, SkinNode, SkinTree,
};
use rekindle_types::dto::skin_tree_role::ViewKey;

const C: ViewKey = ViewKey::Communities;
const WORDS: &str = "Communities";

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
        label: None,
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
    tab.label = Some(label(&tab));
    let id = tab.id.clone();
    strip.insert(last + 1, tab);
    Some(id)
}

/// Whether `n` or a piece of it draws one whole picture `w` px wide: a
/// tab's art with its word drawn in (xslimfire's `TabDownloadButton`, a
/// 65 px "Files"), not a 3-slice whose middle repeats (Alloy's).
fn whole_picture(n: &SkinNode, w: u16) -> bool {
    let own = n.art.as_ref().is_some_and(|a| {
        a.normal.len() == 1
            && a.normal
                .get(&PieceSlot::Middle)
                .is_some_and(|p| p.width == w)
    });
    own || n.children.iter().any(|c| whole_picture(c, w))
}

/// The clone's own words. Its art is the last tab's, word and all.
///
/// - An icon tab (the art one whole picture at the box's size and the box
///   no more than twice as wide as it is tall: Symbiosis's 46 x 48
///   `ButtonVideosTab`, an icon button with no word) gets the shell's
///   Communities glyph, the words its accessible name, and no cover yet: a
///   word would be the only text tab in an icon strip. The cover, the box
///   of the copied icon, is read from the art by [`super::cover`].
/// - A worded whole picture (a word band at least 3.5 times as wide as it
///   is tall: xslimfire's 65 x 18 `TabDownloadButton`, Total Chaos's
///   74 x 17) covers the copied word's band: the box less a tenth of its
///   width (at least 3 px) at each side and a fifth of its height (at least
///   2 px) above and below, which holds the word in both packs
///   (xslimfire's "Files" at x 21-44, y 5-14; Total Chaos's at 21-48, 4-12)
///   and leaves the frame's ends.
/// - Any other tab (a 3-slice whose middle repeats, Alloy's; words over a
///   part, Spacecowboy's) draws the words with no cover.
fn label(tab: &SkinNode) -> NodeLabel {
    let text = WORDS.to_owned();
    match (tab.x, tab.y) {
        (NodeSpan::Fixed { size: w, .. }, NodeSpan::Fixed { size: h, .. })
            if whole_picture(tab, w) =>
        {
            if u32::from(w) <= 2 * u32::from(h) {
                return NodeLabel {
                    text,
                    cover: None,
                    glyph: Some(NodeGlyph::Communities),
                };
            }
            let (dx, dy) = ((w / 10).max(3), (h / 5).max(2));
            NodeLabel {
                text,
                cover: Some([dx, dy, w.saturating_sub(2 * dx), h.saturating_sub(2 * dy)]),
                glyph: None,
            }
        }
        _ => NodeLabel {
            text,
            cover: None,
            glyph: None,
        },
    }
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

/// `span` grown to `[start, start + size)` in a parent `len` long, its
/// anchor kept (a centred strip stays centred as the window grows).
fn regrow(span: NodeSpan, len: i32, start: i32, size: i32) -> NodeSpan {
    let NodeSpan::Fixed { at, .. } = span else {
        return span;
    };
    let size = u16::try_from(size).unwrap_or(u16::MAX);
    let (placed, _) = NodeSpan::Fixed { at, size }.place(len);
    let by = i16::try_from(start - placed).unwrap_or(0);
    let at = match at {
        Anchor::Start(o) => Anchor::Start(o + by),
        Anchor::End(o) => Anchor::End(o - by),
        Anchor::Centre {
            by: b,
            before,
            after,
        } => Anchor::Centre {
            by: b + by,
            before,
            after,
        },
    };
    NodeSpan::Fixed { at, size }
}

/// Fits the node `id` in its strip at `w × h` (rule 3: inside its
/// parent's box). A fixed strip grows over it on each axis, its start kept
/// (Total Chaos's centred 343 px `TabHeader` takes a fourth 74 px tab);
/// a strip that stretches with the window, or the window, cannot, so the
/// run past it across is returned for the window's minimum width.
fn fit(tree: &mut SkinTree, w: i32, h: i32, id: &str) -> i32 {
    let boxes = tree.boxes(w, h);
    let Some(tab) = boxes.iter().find(|(n, _)| n.id == id).map(|b| b.1) else {
        return 0;
    };
    let Some((strip, sb)) = boxes
        .iter()
        .find(|(n, _)| n.children.iter().any(|c| c.id == id))
        .map(|(n, b)| ((*n).clone(), *b))
    else {
        return (tab[0] + tab[2] - w).max(0) + (-tab[0]).max(0);
    };
    let pb = boxes
        .iter()
        .find(|(n, _)| n.children.iter().any(|c| c.id == strip.id))
        .map_or([0, 0, w, h], |b| b.1);
    let mut grown = [strip.x, strip.y];
    let mut across = 0;
    for axis in 0..2 {
        let (s0, s1) = (sb[axis], sb[axis] + sb[axis + 2]);
        let (t0, t1) = (tab[axis], tab[axis] + tab[axis + 2]);
        let run = (t1 - s1).max(0) + (s0 - t0).max(0);
        if run == 0 {
            continue;
        }
        if matches!(grown[axis], NodeSpan::Fixed { .. }) {
            let (a, b) = (s0.min(t0), s1.max(t1));
            grown[axis] = regrow(grown[axis], pb[axis + 2], a - pb[axis], b - a);
        } else if axis == 0 {
            across = run;
        }
    }
    if let Some(n) = find_mut(&mut tree.root, &|n| n.id == strip.id) {
        [n.x, n.y] = grown;
    }
    across
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
        // No art: its words are all it shows.
        label: Some(NodeLabel {
            text: WORDS.to_owned(),
            cover: None,
            glyph: None,
        }),
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
    let keys: Vec<String> = layout.popups.keys().cloned().collect();
    let roots =
        std::iter::once(&mut tree.root).chain(layout.popups.values_mut().map(|p| &mut p.tree.root));
    let mut roots: Vec<&mut Vec<SkinNode>> = roots.collect();
    let mut placed = None;
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
        placed = Some((i, id, packed));
        break;
    }
    if let Some((i, id, packed)) = placed {
        fit_placed(
            layout,
            i.checked_sub(1).map(|p| keys[p].as_str()),
            id.as_deref(),
            packed.as_deref(),
        );
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

/// Fits the placed tab (and a packed row's strip) in its parent, in the
/// main tree (`popup` `None`), whose minimum width grows by a run past a
/// stretching strip, or in a popup, which keeps its own size (StealthX's
/// rail grows its fixed `TabHeader` down).
fn fit_placed(
    layout: &mut WindowLayout,
    popup: Option<&str>,
    id: Option<&str>,
    packed: Option<&str>,
) {
    let out = layout.outsets;
    let (tree, w, h, insets) = match popup.and_then(|k| layout.popups.get_mut(k)) {
        Some(p) => {
            let o = p.tree.outsets;
            (&mut p.tree, p.min_width, p.min_height, o)
        }
        None => match layout.tree.as_mut() {
            Some(t) => (t, layout.min_width, layout.min_height, out),
            None => return,
        },
    };
    let w = i32::from(w) + i32::from(insets.left) + i32::from(insets.right);
    let h = i32::from(h) + i32::from(insets.top) + i32::from(insets.bottom);
    let mut run = 0;
    for id in packed.into_iter().chain(id) {
        run += fit(tree, w, h, id);
    }
    if popup.is_none() {
        if let Ok(run) = u16::try_from(run) {
            layout.min_width = layout.min_width.saturating_add(run);
        }
    }
}
