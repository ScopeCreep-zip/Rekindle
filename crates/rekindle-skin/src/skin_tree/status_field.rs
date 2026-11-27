//! The status field a pack declares but never places (plan SKW3, lead
//! ruling a): every function works in every layout, so the status message
//! and the security alert the status field shows stay reachable. Total
//! Chaos declares `StatusEdit` (`components.xml:898-902`, `Class="StatusView"
//! MinHeight="20"`) and its main window (`mainwindow.xml`) lays no tile for
//! it. Before the walk, the main window gains one tile of that component,
//! just before the views tile: the views tile's `X`, horizontal
//! `Indent*`/`Outdent*`, `Z` and `Y`, stretched on X (`ResizeX="100"`), and
//! the views tile then starts at its bottom (`Y="StatusEdit.BOTTOM"`).
//!
//! The status list arrow is not laid: Total Chaos's `DownArrow`
//! (`components.xml:904-920`) draws `statusarrow.gif`, which its tree does
//! not hold, so there is no art to place; the status choice stays on the
//! File menu and the tray.

use std::collections::BTreeMap;

use rekindle_types::dto::skin::SkinWindow;

use crate::import::tree::Node;
use crate::import::ImportError;

use super::import::bad;

/// The tile name the role table reads as the status field
/// (`roles.rs`, `"statusedit"`).
const NAME: &str = "StatusEdit";

/// The `X`-axis and stacking attributes the status field copies from the
/// views tile.
const COPIED: [&str; 7] = [
    "X",
    "IndentLeft",
    "IndentRight",
    "OutdentLeft",
    "OutdentRight",
    "Z",
    "Y",
];

/// The class of the component `tile` lays, by `by_name` (lower-cased
/// component names).
fn class<'a>(tile: &Node, by_name: &BTreeMap<String, &'a Node>) -> Option<&'a str> {
    let comp = tile.attr_ci("Component")?.trim().to_ascii_lowercase();
    by_name.get(&comp)?.attr_ci("Class").map(str::trim)
}

fn is(class: Option<&str>, want: &[&str]) -> bool {
    class.is_some_and(|c| want.iter().any(|w| c.eq_ignore_ascii_case(w)))
}

fn tiles(node: &Node) -> impl Iterator<Item = &Node> {
    node.children
        .iter()
        .filter(|c| c.name.eq_ignore_ascii_case("Tile"))
}

/// Whether a tile under `node` (or `node` itself) lays a `class` component.
fn lays(node: &Node, want: &[&str], by_name: &BTreeMap<String, &Node>) -> bool {
    is(class(node, by_name), want) || tiles(node).any(|t| lays(t, want, by_name))
}

/// The index path to the views tile under `node`: the tab control holding
/// the buddy list (as `roles.rs` reads it), else the buddy list itself.
fn views_path(node: &Node, by_name: &BTreeMap<String, &Node>) -> Option<Vec<usize>> {
    for (i, child) in node.children.iter().enumerate() {
        if !child.name.eq_ignore_ascii_case("Tile") {
            continue;
        }
        let tabbed = is(class(child, by_name), &["TabControl", "TabbedControl"]);
        if tabbed && tiles(child).any(|t| is(class(t, by_name), &["BuddyListView"])) {
            return Some(vec![i]);
        }
        if is(class(child, by_name), &["BuddyListView"]) {
            return Some(vec![i]);
        }
        if let Some(mut rest) = views_path(child, by_name) {
            rest.insert(0, i);
            return Some(rest);
        }
    }
    None
}

/// `layout` with the status field laid before its views tile, when the
/// pack declares a `StatusView` component in `by_name` and the main
/// window `layout` lays none; `None` when there is nothing to add.
///
/// # Errors
/// A main window with no views tile to place the field before.
pub(super) fn add(
    window: SkinWindow,
    layout: &Node,
    by_name: &BTreeMap<String, &Node>,
) -> Result<Option<Node>, ImportError> {
    const STATUS: &[&str] = &["StatusView"];
    if window != SkinWindow::Main || lays(layout, STATUS, by_name) {
        return Ok(None);
    }
    let Some(component) = by_name
        .values()
        .find(|c| is(c.attr_ci("Class").map(str::trim), STATUS))
        .and_then(|c| c.attr_ci("Name"))
    else {
        return Ok(None);
    };
    let path = views_path(layout, by_name)
        .ok_or_else(|| bad("a declared status field and no views tile to place it before"))?;
    let mut out = layout.clone();
    let (last, parents) = path.split_last().unwrap_or((&0, &[]));
    let parent = parents.iter().fold(&mut out, |n, &i| &mut n.children[i]);
    let views = &mut parent.children[*last];
    let mut attrs: BTreeMap<String, String> = views
        .attrs
        .iter()
        .filter(|(k, _)| COPIED.iter().any(|c| c.eq_ignore_ascii_case(k)))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    attrs.insert("Name".to_owned(), NAME.to_owned());
    attrs.insert("Component".to_owned(), component.trim().to_owned());
    attrs.insert("ResizeX".to_owned(), "100".to_owned());
    views.attrs.retain(|k, _| !k.eq_ignore_ascii_case("Y"));
    views.attrs.insert("Y".to_owned(), format!("{NAME}.BOTTOM"));
    parent.children.insert(
        *last,
        Node {
            name: "Tile".to_owned(),
            attrs,
            ..Node::default()
        },
    );
    Ok(Some(out))
}
