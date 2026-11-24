//! The passes over a walked component tree (plan SK2, design §5
//! "Import"): tab pages that switch together, a single views control split
//! into one view per tab (lead decision 9), an INI pack's button slots
//! turned into one button row per view (lead decision 12), and ids made
//! unique.

use std::collections::BTreeMap;

use rekindle_types::dto::skin_tree::{NodeRole, NodeSpan, SkinNode, ViewKey};

use super::import::bad;
use crate::import::ImportError;

fn node_at<'a>(nodes: &'a mut [SkinNode], path: &[usize]) -> Option<&'a mut SkinNode> {
    let (first, rest) = path.split_first()?;
    let n = nodes.get_mut(*first)?;
    if rest.is_empty() {
        Some(n)
    } else {
        node_at(&mut n.children, rest)
    }
}

/// Tab controls that share a `ControlGroupId` switch together (XGlow's
/// `[TabbedControl]`, `[Add]`, `[Chat]`, `[Join]`, `[Info]`, all
/// `ControlGroupId=1`, `Xglow.ini`): a control's i-th window shows with
/// the views control's i-th view (`[Join]`'s second `JoinGame` is the
/// Servers tab's join).
pub(super) fn pages(root: &mut [SkinNode], groups: &[(Vec<usize>, String)]) {
    let views_of = |n: &SkinNode| -> Vec<ViewKey> {
        n.children
            .iter()
            .filter_map(|c| match c.role {
                NodeRole::View(v) => Some(v),
                _ => None,
            })
            .collect()
    };
    let mut views: BTreeMap<&str, Vec<ViewKey>> = BTreeMap::new();
    for (path, g) in groups {
        let keys = node_at(root, path).map(|n| views_of(n)).unwrap_or_default();
        if !keys.is_empty() {
            views.insert(g, keys);
        }
    }
    for (path, g) in groups {
        let (Some(keys), Some(n)) = (views.get(g.as_str()), node_at(root, path)) else {
            continue;
        };
        if views_of(n).is_empty() {
            for (c, v) in n.children.iter_mut().zip(keys) {
                c.shows_with.get_or_insert(*v);
            }
        }
    }
}

/// Siblings of one name that show with different views (`[Join]`'s two
/// `JoinGame` windows) are named for their view: `JoinGame@Servers`.
pub(super) fn by_view(nodes: &mut [SkinNode]) {
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for n in nodes.iter() {
        *seen.entry(n.id.clone()).or_default() += 1;
    }
    for n in nodes.iter_mut() {
        if let Some(v) = n
            .shows_with
            .filter(|_| seen.get(&n.id).is_some_and(|c| *c > 1))
        {
            n.id = format!("{}@{v:?}", n.id);
        }
        by_view(&mut n.children);
    }
}

/// Makes every id unique: Xfire scopes a tile's name to its parent (Alloy
/// lays a `BvBk` in each of its three views), so a name the tree repeats
/// becomes `Parent.Name`, prefixed again until it is unique.
pub(super) fn qualify(root: &mut [SkinNode]) {
    fn count<'a>(nodes: &'a [SkinNode], seen: &mut BTreeMap<&'a str, usize>) {
        for n in nodes {
            *seen.entry(n.id.as_str()).or_default() += 1;
            count(&n.children, seen);
        }
    }
    fn rename(nodes: &mut [SkinNode], parent: &str, twice: &BTreeMap<String, usize>) -> bool {
        let mut changed = false;
        for n in nodes {
            let own = n.id.clone();
            if twice.get(&own).is_some_and(|c| *c > 1) && !parent.is_empty() {
                n.id = format!("{parent}.{own}");
                changed = true;
            }
            changed |= rename(&mut n.children, &n.id.clone(), twice);
        }
        changed
    }
    for _ in 0..8 {
        let mut seen = BTreeMap::new();
        count(root, &mut seen);
        let twice: BTreeMap<String, usize> = seen
            .into_iter()
            .filter(|(_, c)| *c > 1)
            .map(|(k, c)| (k.to_owned(), c))
            .collect();
        if twice.is_empty() || !rename(root, "", &twice) {
            return;
        }
    }
}

/// Plan SK2 lead decision 9: a pack's single views control holding several
/// tabs (Symbiosis's `XfireView`, `Class="XfireView"`,
/// `Components/MainWindow.xml:132`: one list that every tab of
/// `TabsContainer` switches, `MainWindow.xml:99-125`) is a `Views` node
/// with one `View` per tab, each filling it, in the tabs' document order.
pub(super) fn tab_views(root: &mut [SkinNode]) {
    fn tabs(nodes: &[SkinNode], out: &mut Vec<ViewKey>) {
        for n in nodes {
            if let NodeRole::Tab(v) = n.role {
                if !out.contains(&v) {
                    out.push(v);
                }
            }
            tabs(&n.children, out);
        }
    }
    fn split(nodes: &mut [SkinNode], keys: &[ViewKey]) {
        for n in nodes {
            let bare = !n
                .children
                .iter()
                .any(|c| matches!(c.role, NodeRole::View(_)));
            if n.role == NodeRole::Views && bare {
                let fill = NodeSpan::Stretch { start: 0, end: 0 };
                for v in keys {
                    n.children.push(SkinNode {
                        id: format!("{}.{v:?}", n.id),
                        role: NodeRole::View(*v),
                        x: fill.clone(),
                        y: fill.clone(),
                        z: n.z,
                        art: None,
                        text: None,
                        label: None,
                        inset: None,
                        target: None,
                        fit: None,
                        shows_with: Some(*v),
                        control_group: None,
                        triggers: Vec::new(),
                        children: Vec::new(),
                    });
                }
            }
            split(&mut n.children, keys);
        }
    }
    let mut keys = Vec::new();
    tabs(root, &mut keys);
    split(root, &keys);
}

/// A slot: a group whose every child is a button shown with one view.
fn is_slot(n: &SkinNode) -> bool {
    n.role == NodeRole::Group
        && !n.children.is_empty()
        && n.children
            .iter()
            .all(|c| c.role == NodeRole::Button && c.shows_with.is_some())
}

/// Plan SK2 lead decision 12 (design N11): an INI pack lays its buttons
/// by place, one tab control per slot holding that place's button for
/// each view (Alloy's `[ButtonHolder]` `TabAddButton` = `AddButton` /
/// `AddServerButton` / `SearchFiles`, `ControlGroupId=1`, `Alloy.ini:654-
/// 700`; XGlow's root `[Add]`/`[Chat]`/`[Join]`/`[Info]`; Combat's four
/// round buttons). The XML packs lay them by view (Classic's `ButtonArea`
/// > `FriendsButtons`/`ServersButtons`/`FilesButtons`,
/// `xfiremainwindow.xml:20-42`). The slots of one parent become one
/// `ButtonRow(view)` per view, stretched over that parent; lead decision
/// 13: each slot stays in each of its views' rows as a wrapper (its id,
/// span, z and art: the art Xfire drew behind the button, Combat's 37 px
/// `ChatRefreshStopTab` plate under its 36 px buttons) holding that view's
/// button at its own place in the slot, so every place stays exact. A
/// group holding only slots becomes the `ButtonBar`.
///
/// # Errors
/// A slot's button shown with no view.
pub(super) fn rows_by_view(nodes: &mut Vec<SkinNode>) -> Result<(), ImportError> {
    for n in nodes.iter_mut() {
        rows_by_view(&mut n.children)?;
        let rows_only = !n.children.is_empty()
            && n.children
                .iter()
                .all(|c| matches!(c.role, NodeRole::ButtonRow(_)));
        if n.role == NodeRole::Group && rows_only {
            n.role = NodeRole::ButtonBar;
        }
    }
    let Some(first) = nodes.iter().position(is_slot) else {
        return Ok(());
    };
    let mut rows: Vec<SkinNode> = Vec::new();
    let mut kept = Vec::new();
    for slot in std::mem::take(nodes) {
        if !is_slot(&slot) {
            kept.push(slot);
            continue;
        }
        let mut views: Vec<(ViewKey, SkinNode)> = Vec::new();
        for b in &slot.children {
            let Some(v) = b.shows_with else {
                return Err(bad(format!("{}: a slot's button with no view", b.id)));
            };
            if let Some((_, w)) = views.iter_mut().find(|(w, _)| *w == v) {
                w.children.push(b.clone());
            } else {
                let w = SkinNode {
                    shows_with: Some(v),
                    triggers: Vec::new(),
                    children: vec![b.clone()],
                    ..slot.clone()
                };
                views.push((v, w));
            }
        }
        for (v, w) in views {
            let i = if let Some(i) = rows.iter().position(|r| r.role == NodeRole::ButtonRow(v)) {
                i
            } else {
                let fill = NodeSpan::Stretch { start: 0, end: 0 };
                rows.push(SkinNode {
                    id: format!("{v:?}Buttons"),
                    role: NodeRole::ButtonRow(v),
                    x: fill.clone(),
                    y: fill,
                    z: w.z,
                    art: None,
                    text: None,
                    label: None,
                    inset: None,
                    target: None,
                    fit: None,
                    shows_with: Some(v),
                    control_group: None,
                    triggers: Vec::new(),
                    children: Vec::new(),
                });
                rows.len() - 1
            };
            let row = &mut rows[i];
            row.z = row.z.max(w.z);
            row.children.push(w);
        }
    }
    kept.splice(first..first, rows);
    *nodes = kept;
    Ok(())
}

#[cfg(test)]
mod tests {

    use rekindle_types::dto::skin_action::{SkinAction, SkinEvent};
    use rekindle_types::dto::skin_tree::{Anchor, NodeArt, NodeRole, NodeSpan, ViewKey};

    use super::super::tests::{acts, fixed, n};
    use super::rows_by_view;

    /// Lead decision 12: Alloy's `[ButtonHolder]` slots (`TabAddButton` at
    /// X7 stretched, `TabChatRefreshButton` at X53; `Alloy.ini:654-700`)
    /// become the Friends and Servers rows, each button at its same place.
    #[test]
    fn an_ini_packs_button_slots_are_one_row_per_view() {
        let b = |id: &str, a: SkinAction, v: ViewKey| {
            let mut b = acts(
                n(id, NodeRole::Button, fixed(0, 40), fixed(0, 20)),
                SkinEvent::Click,
                a,
            );
            b.shows_with = Some(v);
            b
        };
        let mut add = n(
            "TabAddButton",
            NodeRole::Group,
            NodeSpan::Stretch { start: 7, end: 0 },
            fixed(3, 20),
        );
        add.children = vec![
            b("AddButton", SkinAction::AddFriend, ViewKey::Friends),
            b("AddServerButton", SkinAction::AddServer, ViewKey::Servers),
        ];
        let mut chat = n(
            "TabChatRefreshButton",
            NodeRole::Group,
            fixed(53, 40),
            fixed(3, 20),
        );
        chat.children = vec![
            b("ChatButton", SkinAction::ChatFriend, ViewKey::Friends),
            b("RefreshButton", SkinAction::RefreshServer, ViewKey::Servers),
        ];
        let mut holder = n(
            "ButtonHolder",
            NodeRole::Group,
            NodeSpan::Fixed {
                at: Anchor::centre(0),
                size: 190,
            },
            fixed(0, 26),
        );
        holder.children = vec![add, chat];
        let mut root = vec![holder];
        rows_by_view(&mut root).unwrap();
        let bar = &root[0];
        assert_eq!(bar.role, NodeRole::ButtonBar);
        // Each row holds its views' slots as wrappers (id, span, z, art),
        // each holding that view's button at its own place in the slot.
        let rows: Vec<_> = bar
            .children
            .iter()
            .map(|r| {
                let slots: Vec<_> = r
                    .children
                    .iter()
                    .map(|w| {
                        let kids: Vec<_> = w
                            .children
                            .iter()
                            .map(|c| (c.id.as_str(), c.x.clone(), c.y.clone()))
                            .collect();
                        (
                            w.id.as_str(),
                            w.role,
                            w.shows_with,
                            w.x.clone(),
                            w.y.clone(),
                            kids,
                        )
                    })
                    .collect();
                (r.role, r.shows_with, slots)
            })
            .collect();
        let add_x = NodeSpan::Stretch { start: 7, end: 0 };
        let own = (fixed(0, 40), fixed(0, 20));
        let slot = |id, v, x, b| {
            (
                id,
                NodeRole::Group,
                Some(v),
                x,
                fixed(3, 20),
                vec![(b, own.0.clone(), own.1.clone())],
            )
        };
        let (f, s) = (ViewKey::Friends, ViewKey::Servers);
        let want = vec![
            (
                NodeRole::ButtonRow(f),
                Some(f),
                vec![
                    slot("TabAddButton", f, add_x.clone(), "AddButton"),
                    slot("TabChatRefreshButton", f, fixed(53, 40), "ChatButton"),
                ],
            ),
            (
                NodeRole::ButtonRow(s),
                Some(s),
                vec![
                    slot("TabAddButton", s, add_x, "AddServerButton"),
                    slot("TabChatRefreshButton", s, fixed(53, 40), "RefreshButton"),
                ],
            ),
        ];
        assert_eq!(rows, want);
    }

    /// Lead decision 13: Combat's centred `ChatRefreshStopTab` (37 px art
    /// over 36 px buttons) has no exact place for its button in the row,
    /// so the slot stays a wrapper with its own span and art, and the
    /// button keeps its place inside it.
    #[test]
    fn a_centred_slot_stays_a_wrapper_with_its_art() {
        let span = NodeSpan::Fixed {
            at: Anchor::centre(0),
            size: 37,
        };
        let mut slot = n("Slot", NodeRole::Group, span.clone(), fixed(0, 20));
        slot.art = Some(NodeArt::middle("slot.png".into(), 37, 20));
        let mut b = acts(
            n("ChatButton", NodeRole::Button, fixed(0, 36), fixed(0, 20)),
            SkinEvent::Click,
            SkinAction::ChatFriend,
        );
        b.shows_with = Some(ViewKey::Friends);
        slot.children = vec![b];
        let mut root = vec![slot];
        rows_by_view(&mut root).unwrap();
        let w = &root[0].children[0];
        let art = w
            .art
            .as_ref()
            .and_then(|a| a.images().next())
            .map(String::as_str);
        assert_eq!(
            (w.id.as_str(), w.x.clone(), art),
            ("Slot", span, Some("slot.png"))
        );
        assert_eq!(
            (w.children[0].id.as_str(), w.children[0].x.clone()),
            ("ChatButton", fixed(0, 36))
        );
    }
}
