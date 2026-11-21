//! The passes over a walked component tree (plan SK2, design §5
//! "Import"): tab pages that switch together, a single views control split
//! into one view per tab (lead decision 9), an INI pack's button slots
//! turned into one button row per view (lead decision 12), and ids made
//! unique.

use std::collections::BTreeMap;

use rekindle_types::dto::skin_tree::{Anchor, NodeRole, NodeSpan, SkinNode, ViewKey};

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
                        x: fill,
                        y: fill,
                        z: n.z,
                        art: None,
                        text: None,
                        shows_with: Some(*v),
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

/// `b`, a span inside a slot laid by `slot`, as the same span inside the
/// slot's parent; `None` when no span says it exactly (a centred place
/// whose rounding differs with the parent's length).
fn compose(slot: NodeSpan, b: NodeSpan) -> Option<NodeSpan> {
    let fixed = |at: Anchor, size: i32| {
        Some(NodeSpan::Fixed {
            at,
            size: u16::try_from(size).ok()?,
        })
    };
    let i = |v: i32| i16::try_from(v).ok();
    match (slot, b) {
        (NodeSpan::Fixed { at, size }, _) => {
            let s = i32::from(size);
            let (b0, bl) = b.place(s);
            match at {
                Anchor::Start(a) => fixed(Anchor::Start(i(i32::from(a) + b0)?), bl),
                Anchor::End(a) => fixed(Anchor::End(i(i32::from(a) + s - b0 - bl)?), bl),
                // ⌊(L−s)/2⌋ = ⌊(L−bl)/2⌋ − (s−bl)/2 only when s−bl is even.
                Anchor::Centre(o) if (s - bl) % 2 == 0 => {
                    fixed(Anchor::Centre(i(i32::from(o) + b0 - (s - bl) / 2)?), bl)
                }
                Anchor::Centre(_) => None,
            }
        }
        (NodeSpan::Stretch { start, end }, NodeSpan::Fixed { at, size }) => {
            let (st, en) = (i32::from(start), i32::from(end));
            let at = match at {
                Anchor::Start(a) => Anchor::Start(i(st + i32::from(a))?),
                Anchor::End(a) => Anchor::End(i(en + i32::from(a))?),
                Anchor::Centre(o) if (st + en) % 2 == 0 => {
                    Anchor::Centre(i(i32::from(o) + (st - en) / 2)?)
                }
                Anchor::Centre(_) => return None,
            };
            fixed(at, i32::from(size))
        }
        (NodeSpan::Stretch { start, end }, NodeSpan::Stretch { start: a, end: c }) => {
            Some(NodeSpan::Stretch {
                start: i(i32::from(start) + i32::from(a))?,
                end: i(i32::from(end) + i32::from(c))?,
            })
        }
    }
}

/// A slot: a group whose every child is a button shown with one view.
fn is_slot(n: &SkinNode) -> bool {
    n.role == NodeRole::Group
        && !n.children.is_empty()
        && n.children
            .iter()
            .all(|c| matches!(c.role, NodeRole::Button(_)) && c.shows_with.is_some())
}

/// Plan SK2 lead decision 12 (design N11): an INI pack lays its buttons
/// by place, one tab control per slot holding that place's button for
/// each view (Alloy's `[ButtonHolder]` `TabAddButton` = `AddButton` /
/// `AddServerButton` / `SearchFiles`, `ControlGroupId=1`, `Alloy.ini:654-
/// 700`; XGlow's root `[Add]`/`[Chat]`/`[Join]`/`[Info]`; Combat's four
/// round buttons). The XML packs lay them by view (Classic's `ButtonArea`
/// > `FriendsButtons`/`ServersButtons`/`FilesButtons`,
/// `xfiremainwindow.xml:20-42`). The slots of one parent become one
/// `ButtonRow(view)` per view, stretched over that parent, each holding
/// that view's buttons at their same places; a group holding only slots
/// becomes the `ButtonBar`.
///
/// # Errors
/// A button whose place in its slot has no exact place in the row.
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
        for mut b in slot.children {
            let Some(v) = b.shows_with else {
                return Err(bad(format!("{}: a slot's button with no view", b.id)));
            };
            let placed = (compose(slot.x, b.x), compose(slot.y, b.y));
            let (Some(x), Some(y)) = placed else {
                return Err(bad(format!(
                    "{}: its place in {} has no exact place in its row",
                    b.id, slot.id
                )));
            };
            (b.x, b.y) = (x, y);
            let i = if let Some(i) = rows.iter().position(|r| r.role == NodeRole::ButtonRow(v)) {
                i
            } else {
                let fill = NodeSpan::Stretch { start: 0, end: 0 };
                rows.push(SkinNode {
                    id: format!("{v:?}Buttons"),
                    role: NodeRole::ButtonRow(v),
                    x: fill,
                    y: fill,
                    z: slot.z,
                    art: None,
                    text: None,
                    shows_with: Some(v),
                    children: Vec::new(),
                });
                rows.len() - 1
            };
            let row = &mut rows[i];
            row.z = row.z.max(slot.z);
            row.children.push(b);
        }
    }
    kept.splice(first..first, rows);
    *nodes = kept;
    Ok(())
}

#[cfg(test)]
mod tests {
    use rekindle_types::dto::skin_tree::{ActionKey, Anchor, NodeRole, NodeSpan, ViewKey};

    use super::super::tests::{fixed, n};
    use super::rows_by_view;

    /// Lead decision 12: Alloy's `[ButtonHolder]` slots (`TabAddButton` at
    /// X7 stretched, `TabChatRefreshButton` at X53; `Alloy.ini:654-700`)
    /// become the Friends and Servers rows, each button at its same place.
    #[test]
    fn an_ini_packs_button_slots_are_one_row_per_view() {
        let b = |id: &str, a: ActionKey, v: ViewKey| {
            let mut b = n(id, NodeRole::Button(a), fixed(0, 40), fixed(0, 20));
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
            b("AddButton", ActionKey::AddFriend, ViewKey::Friends),
            b("AddServerButton", ActionKey::AddServer, ViewKey::Servers),
        ];
        let mut chat = n(
            "TabChatRefreshButton",
            NodeRole::Group,
            fixed(53, 40),
            fixed(3, 20),
        );
        chat.children = vec![
            b("ChatButton", ActionKey::Chat, ViewKey::Friends),
            b("RefreshButton", ActionKey::Refresh, ViewKey::Servers),
        ];
        let mut holder = n(
            "ButtonHolder",
            NodeRole::Group,
            NodeSpan::Fixed {
                at: Anchor::Centre(0),
                size: 190,
            },
            fixed(0, 26),
        );
        holder.children = vec![add, chat];
        let mut root = vec![holder];
        rows_by_view(&mut root).unwrap();
        let bar = &root[0];
        assert_eq!(bar.role, NodeRole::ButtonBar);
        let rows: Vec<_> = bar
            .children
            .iter()
            .map(|r| {
                let kids: Vec<_> = r
                    .children
                    .iter()
                    .map(|c| (c.id.as_str(), c.x, c.y))
                    .collect();
                (r.role, r.shows_with, kids)
            })
            .collect();
        let at = |x: i16| (fixed(x, 40), fixed(3, 20));
        let want = vec![
            (
                NodeRole::ButtonRow(ViewKey::Friends),
                Some(ViewKey::Friends),
                vec![
                    ("AddButton", at(7).0, at(7).1),
                    ("ChatButton", at(53).0, at(53).1),
                ],
            ),
            (
                NodeRole::ButtonRow(ViewKey::Servers),
                Some(ViewKey::Servers),
                vec![
                    ("AddServerButton", at(7).0, at(7).1),
                    ("RefreshButton", at(53).0, at(53).1),
                ],
            ),
        ];
        assert_eq!(rows, want);
    }

    /// A centred slot whose button differs from it by an odd width has no
    /// exact place in a row: the import refuses it rather than guess.
    #[test]
    fn a_button_with_no_exact_place_in_its_row_is_refused() {
        let mut slot = n(
            "Slot",
            NodeRole::Group,
            NodeSpan::Fixed {
                at: Anchor::Centre(0),
                size: 41,
            },
            fixed(0, 20),
        );
        let mut b = n(
            "AddButton",
            NodeRole::Button(ActionKey::AddFriend),
            fixed(0, 40),
            fixed(0, 20),
        );
        b.shows_with = Some(ViewKey::Friends);
        slot.children = vec![b];
        assert!(rows_by_view(&mut vec![slot]).is_err());
    }
}
