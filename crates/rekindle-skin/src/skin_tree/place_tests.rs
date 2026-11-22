//! Rule N4 (plan SK2-P1): every built-in's served main tree holds a
//! reachable Communities view, its control switches to it, and a pack with
//! no tab or knob still gets one.

use rekindle_types::dto::skin::{SkinWindow, WindowLayout};
use rekindle_types::dto::skin_action::{EngineView, NodeTrigger, SkinAction};
use rekindle_types::dto::skin_tree::{NodeRole, NodeSpan, SkinNode, SkinTree};
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
