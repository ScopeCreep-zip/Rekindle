//! The passes over a walked component tree (plan SK2, design §5
//! "Import"): tab pages that switch together, a single views control split
//! into one view per tab (lead decision 9), and ids made unique.

use std::collections::BTreeMap;

use rekindle_types::dto::skin_tree::{NodeRole, NodeSpan, SkinNode, ViewKey};

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
