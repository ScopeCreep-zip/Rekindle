//! The INI era's title strip (plan SK1-S1, lead decision (1)): the skin
//! file's sections read as the XML era's tile tree, laid by
//! [`super::super::xfire_xml::title::add`] with the same rules.
//!
//! An INI section is one component (Alloy `Alloy.ini:265-288`:
//! `[UserNameField]` `Class=Tile`, `Graphics.left/middle/right`,
//! `Graphics.ColorGroup`), and its `Window<n>.<Attr>` keys are its child
//! tiles, each named by `Window<n>.Name` (a section of that name) and
//! placed by `X`, `Y`, `JustX`, `JustY`, `ResizeX`, `IndentLeft`,
//! `IndentRight` as an XML `<Tile>`'s attributes (`[MainWindow]`
//! `Window2.Name=Logo` `X=11` `Y=11`; `[TopButtonBar]` `Window1.Name=
//! MinimizeBox` `X=1` `Y=1`). A state's pieces (`Graphics.*`, `Hover.*`,
//! `BtnDown.*`) are an `<Images State=…>` set with its `ColorGroup` as the
//! set's `Color`; `Text.font/size/bold/color` is a `<Text>`. Repeated keys
//! read the first (as [`super::ini`] does); a tile names no section it is
//! already inside, and the depth is capped.

use std::collections::BTreeMap;

use super::super::tree::Node;
use super::ini::{Ini, Section};

/// The deepest tile nesting read.
const MAX_DEPTH: usize = 8;

/// The tile attributes a `Window<n>.` key may carry.
const TILE_ATTRS: [&str; 9] = [
    "Name",
    "X",
    "Y",
    "Z",
    "JustX",
    "JustY",
    "ResizeX",
    "IndentLeft",
    "IndentRight",
];

/// The state prefixes and the XML state each is.
const SETS: [(&str, &str); 3] = [
    ("Graphics", "Normal"),
    ("Hover", "Hover"),
    ("BtnDown", "Down"),
];

fn node(name: &str, attrs: BTreeMap<String, String>, children: Vec<Node>) -> Node {
    Node {
        name: name.to_owned(),
        attrs,
        text: String::new(),
        children,
    }
}

/// `section`'s child tiles in `Window<n>` order: (n, attrs).
fn windows(section: &Section) -> BTreeMap<u32, BTreeMap<String, String>> {
    let mut out: BTreeMap<u32, BTreeMap<String, String>> = BTreeMap::new();
    for e in &section.entries {
        if !e
            .key
            .get(..6)
            .is_some_and(|p| p.eq_ignore_ascii_case("Window"))
        {
            continue;
        }
        let Some((n, attr)) = e.key[6..].split_once('.') else {
            continue;
        };
        let (Ok(n), Some(attr)) = (
            n.parse::<u32>(),
            TILE_ATTRS.iter().find(|a| a.eq_ignore_ascii_case(attr)),
        ) else {
            continue;
        };
        out.entry(n)
            .or_default()
            .entry((*attr).to_owned())
            .or_insert_with(|| e.value.trim().to_owned());
    }
    out
}

fn tiles(ini: &Ini, section: &Section, path: &mut Vec<String>) -> Vec<Node> {
    let mut out = Vec::new();
    for mut attrs in windows(section).into_values() {
        let Some(name) = attrs.get("Name").cloned() else {
            continue;
        };
        let inside = path.iter().any(|p| p.eq_ignore_ascii_case(&name));
        let children = match ini.section(&name) {
            Some(s) if !inside && path.len() < MAX_DEPTH => {
                path.push(name.clone());
                let c = tiles(ini, s, path);
                path.pop();
                c
            }
            _ => Vec::new(),
        };
        attrs.insert("Component".to_owned(), name);
        out.push(node("Tile", attrs, children));
    }
    out
}

fn component(section: &Section) -> Node {
    let mut children = Vec::new();
    for (prefix, state) in SETS {
        let mut images = Vec::new();
        let mut colour = None;
        for e in &section.entries {
            let Some((p, piece)) = e.key.split_once('.') else {
                continue;
            };
            if !p.eq_ignore_ascii_case(prefix) {
                continue;
            }
            if piece.eq_ignore_ascii_case("ColorGroup") {
                colour.get_or_insert_with(|| e.value.trim().to_owned());
            } else if !piece.eq_ignore_ascii_case("ColorGroupBG") {
                images.push(Node {
                    name: "Image".to_owned(),
                    attrs: BTreeMap::from([("Type".to_owned(), piece.to_owned())]),
                    text: e.value.trim().to_owned(),
                    children: Vec::new(),
                });
            }
        }
        if images.is_empty() {
            continue;
        }
        let mut attrs = BTreeMap::from([("State".to_owned(), state.to_owned())]);
        if let Some(c) = colour {
            attrs.insert("Color".to_owned(), c);
        }
        children.push(node("Images", attrs, images));
    }
    let text: BTreeMap<String, String> = [
        ("font", "Font"),
        ("size", "Size"),
        ("bold", "Bold"),
        ("color", "Color"),
    ]
    .into_iter()
    .filter_map(|(k, a)| {
        Some((
            a.to_owned(),
            section.get(&format!("Text.{k}"))?.trim().to_owned(),
        ))
    })
    .collect();
    if text.contains_key("Font") {
        children.push(node("Text", text, Vec::new()));
    }
    let mut attrs = BTreeMap::from([("Name".to_owned(), section.name.clone())]);
    if let Some(class) = section.get("Class") {
        attrs.insert("Class".to_owned(), class.trim().to_owned());
    }
    node("Component", attrs, children)
}

/// The main window's tile tree (rooted at `[MainWindow]`) and every
/// section as a component, as [`super::super::xfire_xml::title::add`]
/// reads them; `None` without a `[MainWindow]`.
#[must_use]
pub fn tree(skin: &Ini) -> Option<(Node, Node)> {
    let main = skin.section("MainWindow")?;
    let mut path = vec![main.name.clone()];
    let layout = node("Tile", BTreeMap::new(), tiles(skin, main, &mut path));
    let components = node(
        "Components",
        BTreeMap::new(),
        skin.sections.iter().map(component).collect(),
    );
    Some((layout, components))
}
