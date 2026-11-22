//! Rule N4 (plan SK2-P1): every built-in's served main tree holds a
//! reachable Communities view, its control switches to it, and a pack with
//! no tab or knob still gets one.

use rekindle_types::dto::skin::{SkinWindow, WindowLayout};
use rekindle_types::dto::skin_action::{EngineView, NodeTrigger, SkinAction};
use rekindle_types::dto::skin_tree::{
    NodeLabel, NodeRole, NodeSpan, ShellNode, SkinNode, SkinTree,
};
use rekindle_types::dto::skin_tree_role::ViewKey;

use super::place::communities;
use super::test_support::{frame, Z};
use crate::{builtin, manifest, package};

const C: ViewKey = ViewKey::Communities;

fn all<'a>(nodes: &'a [SkinNode], out: &mut Vec<&'a SkinNode>) {
    for n in nodes {
        out.push(n);
        all(&n.children, out);
    }
}

/// The views node, and the page of the Communities view in it.
fn views_page(root: &[SkinNode]) -> (Option<u16>, usize) {
    let mut nodes = Vec::new();
    all(root, &mut nodes);
    let views = nodes
        .into_iter()
        .find(|n| n.role == NodeRole::Views)
        .expect("a views node");
    let page = views
        .children
        .iter()
        .position(|c| c.role == NodeRole::View(C))
        .expect("View(Communities) beside the other views");
    assert_eq!(views.children[page].shows_with, Some(C));
    (views.control_group, page)
}

/// The view `run` leaves shown, as the shell's tree operations apply it
/// (`signalEffects.ts` `applyTreeOp`): `ShowView`, then a `GroupPage` of
/// the views group to a view's page.
fn shown_after(
    triggers: &[NodeTrigger],
    views: (Option<u16>, usize),
    root: &[SkinNode],
) -> Option<ViewKey> {
    let mut shown = None;
    for t in triggers.iter().filter(|t| t.presses()) {
        match &t.run {
            SkinAction::ShowView(EngineView::Communities) => shown = Some(C),
            SkinAction::GroupPage { group, page } if Some(*group) == views.0 => {
                let mut nodes = Vec::new();
                all(root, &mut nodes);
                let v = nodes.into_iter().find(|n| n.role == NodeRole::Views)?;
                if let Some(NodeRole::View(k)) = v.children.get(usize::from(*page)).map(|c| c.role)
                {
                    shown = Some(k);
                }
            }
            _ => {}
        }
    }
    shown
}

fn served_main(id: &str) -> WindowLayout {
    let (_, b) = builtin::find(&format!("builtin:{id}")).unwrap();
    let mut m = manifest::served(&package::read(b.bytes).unwrap()).unwrap();
    m.windows.remove(&SkinWindow::Main).unwrap()
}

/// The pressable nodes of the main tree and its popups that leave the
/// Communities view shown.
fn openers(main: &WindowLayout) -> Vec<(String, NodeRole)> {
    let root = &main.tree.as_ref().unwrap().root;
    let views = views_page(root);
    let mut nodes = Vec::new();
    all(root, &mut nodes);
    for p in main.popups.values() {
        all(&p.tree.root, &mut nodes);
    }
    nodes
        .into_iter()
        .filter(|n| n.role.interactive(&n.triggers))
        .filter(|n| shown_after(&n.triggers, views, root) == Some(C))
        .map(|n| (n.id.clone(), n.role))
        .collect()
}

#[test]
fn every_builtin_serves_a_reachable_communities_view() {
    for b in &builtin::PACKS {
        let main = served_main(b.id);
        let open = openers(&main);
        assert!(
            !open.is_empty(),
            "{}: nothing opens the Communities view",
            b.id
        );
        for (id, role) in &open {
            assert!(
                matches!(role, NodeRole::Tab(_) | NodeRole::Knob(_)),
                "{}: {id} opens Communities as {role:?}",
                b.id
            );
        }
        let mut nodes = Vec::new();
        all(&main.tree.as_ref().unwrap().root, &mut nodes);
        for p in main.popups.values() {
            all(&p.tree.root, &mut nodes);
        }
        let controls = nodes
            .iter()
            .filter(|n| matches!(n.role, NodeRole::Tab(k) | NodeRole::Knob(k) if k == C))
            .count();
        assert_eq!(controls, 1, "{}: one Communities control", b.id);
        // Every built-in has a native slot (tabs, Combat's knob, StealthX's
        // rail tabs): none needs the Host's own tab.
        assert!(
            nodes.iter().all(|n| !n.id.ends_with(".CommunitiesTab")),
            "{}: a native slot is skipped",
            b.id
        );
    }
}

#[test]
fn a_tab_pack_gets_a_cloned_tab_that_switches_and_a_knob_pack_a_stop() {
    // Classic: three tabs over a paged views group (`TabbedControl`, group 1).
    let classic = served_main("xfire-classic");
    let root = &classic.tree.as_ref().unwrap().root;
    let mut nodes = Vec::new();
    all(root, &mut nodes);
    let tab = nodes
        .iter()
        .find(|n| n.role == NodeRole::Tab(C))
        .expect("Classic's Communities tab");
    assert_eq!(tab.id, "TabDownloadText.Communities");
    assert!(
        tab.art.is_some() || !tab.children.is_empty(),
        "the last tab's art is cloned"
    );
    assert_eq!(shown_after(&tab.triggers, views_page(root), root), Some(C));
    // StealthX: the tabs ride the `MainMenuPopup` rail.
    let stealth = served_main("stealthx");
    assert!(stealth.popups.values().any(|p| {
        let mut n = Vec::new();
        all(&p.tree.root, &mut n);
        n.iter().any(|n| n.role == NodeRole::Tab(C))
    }));
    // Combat: the last stop turns to the new one.
    let combat = served_main("combat");
    let open = openers(&combat);
    assert_eq!(
        open.iter().map(|(_, r)| *r).collect::<Vec<_>>(),
        [NodeRole::Knob(ViewKey::Files)]
    );
}

fn bare(views_group: Option<u16>) -> WindowLayout {
    let fill = NodeSpan::Stretch { start: 4, end: 4 };
    let views = SkinNode {
        id: "List".to_owned(),
        role: NodeRole::Views,
        x: fill,
        y: NodeSpan::Stretch { start: 40, end: 20 },
        z: 1,
        art: None,
        text: None,
        label: None,
        shows_with: None,
        control_group: views_group,
        triggers: Vec::new(),
        children: Vec::new(),
    };
    let mut l = frame();
    l.tree = Some(SkinTree {
        outsets: Z,
        root: vec![views],
    });
    l
}

#[test]
fn a_pack_with_no_tab_or_knob_gets_a_host_tab() {
    for group in [None, Some(3)] {
        let mut l = bare(group);
        communities(&mut l);
        let root = &l.tree.as_ref().unwrap().root;
        let roles: Vec<NodeRole> = root[0].children.iter().map(|c| c.role).collect();
        assert_eq!(roles, [NodeRole::View(ViewKey::Friends), NodeRole::View(C)]);
        let open = openers(&l);
        assert_eq!(open, [("List.CommunitiesTab".to_owned(), NodeRole::Tab(C))]);
        // Placed once: a second read leaves the tree as it is.
        let before = l.clone();
        communities(&mut l);
        assert_eq!(l, before);
    }
}

/// StealthX's popup rail tab (plan SK2-N4): its press is the launcher's
/// view switch, `ShowView(Communities)` and the main tree's views page,
/// and the popup holds no views, so the window keeps both for its own art
/// (its `ButtonArea` pages with group 1: a pack's groups are skin-wide)
/// and the Host addresses both to the launcher (`applyTreeOp`'s
/// `holdsViews`).
#[test]
fn stealthx_popup_tab_is_its_launchers_view_switch() {
    let stealth = served_main("stealthx");
    let main_root = &stealth.tree.as_ref().unwrap().root;
    let (group, page) = views_page(main_root);
    let group = group.expect("StealthX's views page as group 1");
    let popup = stealth
        .popups
        .values()
        .find(|p| {
            let mut n = Vec::new();
            all(&p.tree.root, &mut n);
            n.iter().any(|n| n.role == NodeRole::Tab(C))
        })
        .expect("the rail popup");
    let mut nodes = Vec::new();
    all(&popup.tree.root, &mut nodes);
    let tab = nodes.iter().find(|n| n.role == NodeRole::Tab(C)).unwrap();
    let presses: Vec<&SkinAction> = tab
        .triggers
        .iter()
        .filter(|t| t.presses())
        .map(|t| &t.run)
        .collect();
    assert!(presses.contains(&&SkinAction::ShowView(EngineView::Communities)));
    let page = u16::try_from(page).unwrap();
    assert!(presses.contains(&&SkinAction::GroupPage { group, page }));
    assert!(nodes.iter().all(|n| n.role != NodeRole::Views));
    assert!(nodes.iter().any(|n| n.control_group == Some(group)));
}

/// Whether the node at `path` (indices from the root) shows under the
/// shell's rules (`SkinTreeWindow.tsx` `offPage`/`offView`): each node is
/// its paged parent's current page, and a node tied to a view shows with it.
fn shows(
    root: &[SkinNode],
    path: &[usize],
    view: Option<ViewKey>,
    pages: &std::collections::BTreeMap<u16, u16>,
) -> bool {
    let mut list = root;
    let mut group = None;
    for &i in path {
        let n = &list[i];
        if let Some(g) = group {
            if usize::from(pages.get(&g).copied().unwrap_or(0)) != i {
                return false;
            }
        }
        let tied = n.shows_with.or(match n.role {
            NodeRole::View(k) => Some(k),
            _ => None,
        });
        if tied.is_some() && tied != view {
            return false;
        }
        group = n.control_group;
        list = &n.children;
    }
    true
}

fn path_to(list: &[SkinNode], f: &dyn Fn(&SkinNode) -> bool) -> Option<Vec<usize>> {
    for (i, n) in list.iter().enumerate() {
        if f(n) {
            return Some(vec![i]);
        }
        if let Some(mut p) = path_to(&n.children, f) {
            p.insert(0, i);
            return Some(p);
        }
    }
    None
}

/// StealthX (walk 2, plan SK2-N4): the popup rail's tab is pressed and the
/// Host addresses its view switch to the buddy list, which applies each
/// press as the shell does (`applyTreeOp`); the main tree's
/// `View(Communities)` then shows and `View(Friends)` does not.
#[test]
fn stealthx_main_shows_communities_on_the_addressed_switch() {
    let stealth = served_main("stealthx");
    let root = &stealth.tree.as_ref().unwrap().root;
    let tab = stealth
        .popups
        .values()
        .find_map(|p| {
            let mut n = Vec::new();
            all(&p.tree.root, &mut n);
            n.into_iter().find(|n| n.role == NodeRole::Tab(C)).cloned()
        })
        .expect("the rail's Communities tab");
    let (mut view, mut pages) = (None, std::collections::BTreeMap::new());
    for t in tab.triggers.iter().filter(|t| t.presses()) {
        match t.run {
            SkinAction::ShowView(EngineView::Communities) => view = Some(C),
            SkinAction::GroupPage { group, page } => {
                pages.insert(group, page);
            }
            _ => {}
        }
    }
    let at = |k: ViewKey| path_to(root, &|n| n.role == NodeRole::View(k)).unwrap();
    assert!(shows(root, &at(C), view, &pages), "view:communities hidden");
    assert!(!shows(root, &at(ViewKey::Friends), view, &pages));
}

/// The Host's control and its parent's box, in each tree of `main` at its
/// minimum size.
fn placed_boxes(main: &WindowLayout) -> Vec<(String, [i32; 4], [i32; 4])> {
    // The window is its minimum plus the layout's outsets (Symbiosis's
    // 13 px right overhang); a popup's are its tree's.
    let tree = main.tree.as_ref().unwrap();
    let mut trees = vec![(tree, main.min_width, main.min_height, main.outsets)];
    trees.extend(
        main.popups
            .values()
            .map(|p| (&p.tree, p.min_width, p.min_height, p.tree.outsets)),
    );
    let mut out = Vec::new();
    for (t, w, h, o) in trees {
        let w = i32::from(w) + i32::from(o.left) + i32::from(o.right);
        let h = i32::from(h) + i32::from(o.top) + i32::from(o.bottom);
        let boxes = t.boxes(w, h);
        let t_o = t.outsets;
        let pack = [
            i32::from(t_o.left),
            i32::from(t_o.top),
            w - i32::from(t_o.left) - i32::from(t_o.right),
            h - i32::from(t_o.top) - i32::from(t_o.bottom),
        ];
        for (n, b) in &boxes {
            if !matches!(n.role, NodeRole::Tab(C) | NodeRole::Knob(C)) {
                continue;
            }
            let parent = boxes
                .iter()
                .find(|(p, _)| p.children.iter().any(|c| c.id == n.id))
                .map_or(pack, |p| p.1);
            out.push((n.id.clone(), *b, parent));
        }
    }
    out
}

/// Every built-in's Communities control lies inside its parent's box
/// (walk 2: Total Chaos's clone ran 14 px past its 343 px `TabHeader`).
#[test]
fn every_builtin_places_its_communities_control_inside_its_parent() {
    for b in &builtin::PACKS {
        let placed = placed_boxes(&served_main(b.id));
        assert_eq!(placed.len(), 1, "{}: {placed:?}", b.id);
        let (id, c, p) = &placed[0];
        let inside = c[0] >= p[0]
            && c[1] >= p[1]
            && c[0] + c[2] <= p[0] + p[2]
            && c[1] + c[3] <= p[1] + p[3];
        assert!(inside, "{}: {id} {c:?} outside its parent {p:?}", b.id);
    }
}

/// xslimfire's and Total Chaos's tabs are one picture with their word
/// drawn in: the clone carries its own words and covers the copied one;
/// the pack's own tab carries none.
#[test]
fn an_art_label_clone_carries_its_own_label() {
    for id in ["xslimfire", "total-chaos"] {
        let main = served_main(id);
        let mut nodes = Vec::new();
        all(&main.tree.as_ref().unwrap().root, &mut nodes);
        let clone = nodes.iter().find(|n| n.role == NodeRole::Tab(C)).unwrap();
        let source = nodes
            .iter()
            .find(|n| n.role == NodeRole::Tab(ViewKey::Files))
            .unwrap();
        assert_eq!(source.label, None, "{id}");
        let Some(NodeLabel {
            text,
            cover: Some(cover),
        }) = &clone.label
        else {
            panic!("{id}: {:?}", clone.label);
        };
        assert_eq!(text, "Communities", "{id}");
        let (NodeSpan::Fixed { size: w, .. }, NodeSpan::Fixed { size: h, .. }) = (clone.x, clone.y)
        else {
            panic!("{id}: a fixed tab");
        };
        assert!(
            cover[0] + cover[2] <= w && cover[1] + cover[3] <= h,
            "{id}: {cover:?}"
        );
        assert!(
            cover[2] * 2 > w && cover[3] * 2 > h,
            "{id}: {cover:?} covers the word band"
        );
        // The window reads the served shell: the label rides it.
        let shell = main
            .tree
            .as_ref()
            .unwrap()
            .shell(SkinWindow::Main, (0, 0), Vec::new());
        let mut stack: Vec<&ShellNode> = shell.root.iter().collect();
        let mut found = None;
        while let Some(n) = stack.pop() {
            if n.id == clone.id {
                found = Some(n);
            }
            stack.extend(n.children.iter());
        }
        assert_eq!(found.unwrap().label, clone.label, "{id}: shell label");
    }
}
