//! Plan E7.7 (round 32, finding SR1, owner: "every Xfire button works in
//! every layout"): on the Servers view every Servers-row button is shown,
//! and on Friends every Friends-row button, under the shell's rules
//! (`SkinTreeWindow.tsx` `offPage`/`offView`).

use std::collections::BTreeMap;

use rekindle_types::dto::skin_action::{EngineView, SkinAction, SkinEvent};
use rekindle_types::dto::skin_tree::{NodeRole, SkinNode};
use rekindle_types::dto::skin_tree_role::ViewKey;

use super::golden_tests::main_layout;

const BUILTINS: [&str; 11] = [
    "symbiosis",
    "xfire-classic",
    "combat",
    "alloy",
    "onyx",
    "simple-x",
    "spacecowboy",
    "stealthx",
    "total-chaos",
    "xglow",
    "xslimfire",
];

/// The pages showing `v` sets, group by group: every `Show(<v>)` page and
/// every press page of `v`'s tab or knob (and the parts inside it).
fn pages_of(
    nodes: &[SkinNode],
    v: ViewKey,
    e: EngineView,
    tab: bool,
    out: &mut BTreeMap<u16, Vec<u16>>,
) {
    for n in nodes {
        let tab = tab || matches!(n.role, NodeRole::Tab(k) | NodeRole::Knob(k) if k == v);
        for t in &n.triggers {
            if let SkinAction::GroupPage { group, page } = t.run {
                if t.on == SkinEvent::Show(e) || (tab && t.presses()) {
                    out.entry(group).or_default().push(page);
                }
            }
        }
        pages_of(&n.children, v, e, tab, out);
    }
}

/// Every node in `v`'s row hidden on `v`'s view, with why.
fn hidden(
    nodes: &[SkinNode],
    up: (Option<u16>, Option<u16>),
    in_row: bool,
    v: ViewKey,
    pages: &BTreeMap<u16, Vec<u16>>,
    out: &mut Vec<String>,
) {
    for (i, n) in nodes.iter().enumerate() {
        let in_row = in_row || n.role == NodeRole::ButtonRow(v);
        if in_row {
            if let (Some(g), grand) = up {
                if grand != Some(g) {
                    let at = pages.get(&g).cloned().unwrap_or_else(|| vec![0]);
                    if at.iter().any(|&p| usize::from(p) != i) {
                        out.push(format!("{}: child {i} of group {g}, paged to {at:?}", n.id));
                    }
                }
            }
            if n.shows_with.is_some_and(|s| s != v) {
                out.push(format!("{}: shows with {:?}", n.id, n.shows_with));
            }
        }
        hidden(&n.children, (n.control_group, up.0), in_row, v, pages, out);
    }
}

/// Real-app round 32 (Alloy, Simple X, Combat): every Servers-row button
/// was hidden on the Servers view, each wrapper still paged by the views'
/// group at a page the Servers tab never shows. Symbiosis's row stays the
/// page its bar shows (CB2), and Friends keeps its row.
#[test]
fn every_row_button_shows_on_its_own_view() {
    let mut rows = 0;
    let mut bad = Vec::new();
    for id in BUILTINS {
        let main = main_layout(id);
        let roots: Vec<&[SkinNode]> = std::iter::once(main.tree.as_ref().unwrap().root.as_slice())
            .chain(main.popups.values().map(|p| p.tree.root.as_slice()))
            .collect();
        for (v, e) in [
            (ViewKey::Servers, EngineView::Servers),
            (ViewKey::Friends, EngineView::Friends),
        ] {
            let mut pages = BTreeMap::new();
            for r in &roots {
                pages_of(r, v, e, false, &mut pages);
            }
            for r in &roots {
                let mut out = Vec::new();
                hidden(r, (None, None), false, v, &pages, &mut out);
                bad.extend(out.into_iter().map(|o| format!("{id} {v:?}: {o}")));
            }
            let has_row = |n: &SkinNode| n.role == NodeRole::ButtonRow(v);
            if v == ViewKey::Servers
                && roots
                    .iter()
                    .any(|r| super::community_row::find(r, &has_row).is_some())
            {
                rows += 1;
            }
        }
    }
    assert!(
        bad.is_empty(),
        "hidden on their own view:\n{}",
        bad.join("\n")
    );
    assert!(rows >= 4, "only {rows} packs with a Servers row");
}
