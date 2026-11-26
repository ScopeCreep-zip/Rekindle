//! Plan E7.7 (round 32, finding SR1): a view's button row shows every
//! button it holds while its view is shown.
//!
//! An INI pack lays one slot per place holding that place's button for each
//! view, the slot a page of the views' control group (Alloy's
//! `TabAddButton` = `AddButton` / `AddServerButton` / `SearchFiles`,
//! `ControlGroupId=1`, `Alloy.ini:654-700`; Combat's round buttons,
//! `combat.ini`). The import splits each slot into one wrapper per view
//! row (lead decision 13, [`super::passes`] `rows_by_view`), each wrapper
//! holding only its own view's button, at index 0. A wrapper that kept the
//! group showed its button only while the group stood at page 0 (the
//! shell's `offPage`, `SkinTreeWindow.tsx`), so the Servers tab, which
//! pages group 1 to 1, hid every Servers button (Combat's knob pages group
//! 1 to 1 on Friends and 2 on Servers, hiding both rows). The row's own
//! view already shows it, so the wrapper is not paged by the views' group.
//! A row that is itself a page of the group (Symbiosis's
//! `ServersButtonBar` under `TabsButtonsContainer`) and a slot paged by a
//! group the views do not page (a state toggle) are left.

use std::collections::BTreeSet;

use rekindle_types::dto::skin::WindowLayout;
use rekindle_types::dto::skin_action::{SkinAction, SkinEvent};
use rekindle_types::dto::skin_tree::{NodeRole, SkinNode};

/// The groups that page with the views: a `Views` node's group, and every
/// group a `Show(<view>)` pages.
fn view_groups(nodes: &[SkinNode], out: &mut BTreeSet<u16>) {
    for n in nodes {
        if n.role == NodeRole::Views {
            out.extend(n.control_group);
        }
        for t in &n.triggers {
            if let (SkinEvent::Show(_), SkinAction::GroupPage { group, .. }) = (&t.on, &t.run) {
                out.insert(*group);
            }
        }
        view_groups(&n.children, out);
    }
}

/// No direct child of a button row pages with a views group its row is
/// not itself a page of.
fn unpage_rows(nodes: &mut [SkinNode], views: &BTreeSet<u16>) {
    for n in nodes {
        if matches!(n.role, NodeRole::ButtonRow(_)) {
            let own = n.control_group;
            for c in &mut n.children {
                if c.control_group
                    .is_some_and(|g| views.contains(&g) && own != Some(g))
                {
                    c.control_group = None;
                }
            }
        }
        unpage_rows(&mut n.children, views);
    }
}

/// Every button row of `layout`'s main tree and popups shows its buttons
/// with its view (see the module docs).
pub fn rows(layout: &mut WindowLayout) {
    let mut roots: Vec<&mut Vec<SkinNode>> = layout
        .tree
        .as_mut()
        .map(|t| &mut t.root)
        .into_iter()
        .chain(layout.popups.values_mut().map(|p| &mut p.tree.root))
        .collect();
    let mut views = BTreeSet::new();
    for r in &roots {
        view_groups(r, &mut views);
    }
    for r in &mut roots {
        unpage_rows(r, &views);
    }
}
