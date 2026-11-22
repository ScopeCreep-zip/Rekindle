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
//! read the first (as [`super::ini`] does), except a position key set to a
//! sibling's edge and then to a number: the number is the offset from that
//! edge (XGlow's `Window49.Y=Bottom.BOTTOM` `Window49.Y=-52` is
//! `Bottom.BOTTOM-52`; its four buttons `X=ButtonDivider.LEFT` then
//! `-104/-52/0/62` stand side by side as in `skinref/xglow`, and
//! `Window40.Y=Middle.BOTTOM` `Y=0` keeps `Bottom` at the window's bottom
//! as the original draws it, where a last-wins `Y=0` would lay it at the
//! top; plan SK2 lead decision 4). A tile names no section it is
//! already inside, and the depth is capped.

use std::collections::BTreeMap;

use super::super::tree::Node;
use super::ini::{Ini, Section};

/// The deepest tile nesting read.
const MAX_DEPTH: usize = 8;

/// The tile attributes a `Window<n>.` key may carry.
const TILE_ATTRS: [&str; 16] = [
    "Name",
    "X",
    "Y",
    "Z",
    "JustX",
    "JustY",
    "ResizeX",
    "IndentLeft",
    "IndentRight",
    "ResizeY",
    "IndentTop",
    "IndentBottom",
    "OutdentLeft",
    "OutdentRight",
    "OutdentTop",
    "OutdentBottom",
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
        put(out.entry(n).or_default(), attr, e.value.trim());
    }
    out
}

/// Sets `attr` to `value` in `attrs`: the first value wins, except a
/// sibling's edge then a number ([`offset_of`]).
fn put(attrs: &mut BTreeMap<String, String>, attr: &str, value: &str) {
    attrs
        .entry(attr.to_owned())
        .and_modify(|first: &mut String| {
            if let Some(joined) = offset_of(attr, first, value) {
                *first = joined;
            }
        })
        .or_insert_with(|| value.to_owned());
}

/// A repeated `X`/`Y` whose first value is a sibling's edge
/// (`Bottom.BOTTOM`) and whose second is a number: the edge plus it.
fn offset_of(attr: &str, first: &str, value: &str) -> Option<String> {
    let n: i32 = value.parse().ok()?;
    let edge = (attr == "X" || attr == "Y")
        && first.contains('.')
        && !first.contains(['+', '-'])
        && first.parse::<i32>().is_err();
    edge.then(|| format!("{first}{n:+}"))
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

/// The INI state name a `<state>.Trigger=` key selects, as the XML
/// `State(...)` argument: `Graphics`/`Text` are the normal look, `BtnDown`
/// the pressed one. Any other name passes through and the tree's trigger
/// reader refuses it (`ImportError::UnknownAction`).
fn state_of(prefix: &str) -> &str {
    if prefix.eq_ignore_ascii_case("Graphics") || prefix.eq_ignore_ascii_case("Text") {
        "Normal"
    } else if prefix.eq_ignore_ascii_case("BtnDown") {
        "Down"
    } else {
        prefix
    }
}

/// Whether `key` is a trigger key: `Action.<event>` or `<state>.Trigger`.
pub(super) fn is_trigger_key(key: &str) -> bool {
    key.split_once('.')
        .is_some_and(|(p, k)| p.eq_ignore_ascii_case("Action") || k.eq_ignore_ascii_case("Trigger"))
}

/// `section`'s `Action.<event>=A` and `<state>.Trigger=<signal>` keys as
/// the XML era's `<Triggers>` (plan SK2-A, decision 36), in the file's
/// order: every duplicate key is kept (Combat's `[InfoViewSwitchIn]`
/// runs `GroupMessage(1)` then `LaunchSkin`, its own comment says in that
/// order, `combat.ini:804-807`).
fn triggers(section: &Section) -> Option<Node> {
    let trigger = |event: String, action: String| Node {
        name: "Trigger".to_owned(),
        attrs: BTreeMap::from([("Type".to_owned(), event)]),
        text: action,
        children: Vec::new(),
    };
    let on = |e: &str| {
        let rest = if e.len() > 2 && e[..2].eq_ignore_ascii_case("on") {
            &e[2..]
        } else {
            e
        };
        format!("On{rest}")
    };
    let list: Vec<Node> = section
        .entries
        .iter()
        .filter(|e| is_trigger_key(&e.key))
        .filter_map(|e| {
            let (p, k) = e.key.split_once('.')?;
            let v = e.value.trim().to_owned();
            Some(if p.eq_ignore_ascii_case("Action") {
                trigger(on(k.trim()), v)
            } else {
                trigger(on(&v), format!("State({})", state_of(p.trim())))
            })
        })
        .collect();
    (!list.is_empty()).then(|| node("Triggers", BTreeMap::new(), list))
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
    if let Some(t) = triggers(section) {
        children.push(t);
    }
    // A `SkinLauncher`'s `Skin.Name=` is the popup its `LaunchSkin` opens
    // (`Skin.Name=Infoskin`, SimpleX's `[InfoViewButton]`).
    if let Some(skin) = section.get("Skin.Name").or(section.get("Skin")) {
        children.push(Node {
            name: "Popup".to_owned(),
            attrs: BTreeMap::new(),
            text: skin.trim().to_owned(),
            children: Vec::new(),
        });
    }
    let mut attrs = BTreeMap::from([("Name".to_owned(), section.name.clone())]);
    if let Some(class) = section.get("Class") {
        attrs.insert("Class".to_owned(), class.trim().to_owned());
    }
    // `ControlGroupId`: the tab controls that switch together (plan SK2
    // lead decision 6: XGlow's `[Join]` pages show with the views).
    // `DefaultWidth`: a popup's width (`[Infoskin]` `DefaultWidth=150`,
    // SimpleX `Infoskin.ini:3`), as the XML era's component attribute.
    for key in [
        "DefaultWidth",
        "MinWidth",
        "MinHeight",
        "MaxWidth",
        "MaxHeight",
        "ControlGroupId",
        "GroupId",
    ] {
        if let Some(v) = section.get(key) {
            attrs.insert(key.to_owned(), v.trim().to_owned());
        }
    }
    node("Component", attrs, children)
}

/// The tile tree rooted at section `root` (`[MainWindow]` for the main
/// window, `[Infoskin]` for SimpleX's popup file) and every section of
/// `skin` as a component, as [`super::super::xfire_xml::title::add`] reads
/// them; `None` without a `[root]`.
#[must_use]
pub fn tree(skin: &Ini, root: &str) -> Option<(Node, Node)> {
    let main = skin.section(root)?;
    let mut path = vec![main.name.clone()];
    let layout = node("Tile", BTreeMap::new(), tiles(skin, main, &mut path));
    let components = node(
        "Components",
        BTreeMap::new(),
        skin.sections.iter().map(component).collect(),
    );
    Some((layout, components))
}

/// The `<Popups>` node of the popup file `file` that `launcher`'s
/// `Skin.Name=name` opens (INI popups are separate files: SimpleX's
/// `[InfoViewButton]` `Skin.Name=Infoskin`, `SimpleX.ini:453`, opens
/// `Infoskin.ini`, rooted at its `[Infoskin]`): one tile named `name`,
/// placed by the launcher's `Skin.<attr>` keys as a `Window<n>.<attr>`
/// (`Skin.X=Middle.RIGHT` `Skin.X=-8` is `Middle.RIGHT-8`,
/// `SimpleX.ini:457-458`) and stretched by the root section's `ResizeY`,
/// beside the file's sections as the popup's own components (XGlow's
/// popup `[Top]`, `XGlowInfoSkin.ini:98`, is not its main window's `[Top]`);
/// `None` without a `[name]`.
#[must_use]
pub fn popup(launcher: &Section, name: &str, file: &Ini) -> Option<Node> {
    let root = file.section(name)?;
    let (mut layout, components) = tree(file, name)?;
    for e in &launcher.entries {
        let Some((p, attr)) = e.key.split_once('.') else {
            continue;
        };
        let known = TILE_ATTRS
            .iter()
            .find(|a| a.eq_ignore_ascii_case(attr) && **a != "Name");
        if let (true, Some(attr)) = (p.eq_ignore_ascii_case("Skin"), known) {
            put(&mut layout.attrs, attr, e.value.trim());
        }
    }
    if let Some(v) = root.get("ResizeY") {
        put(&mut layout.attrs, "ResizeY", v.trim());
    }
    layout.attrs.insert("Name".to_owned(), name.to_owned());
    layout
        .attrs
        .insert("Component".to_owned(), root.name.clone());
    Some(node("Popups", BTreeMap::new(), vec![layout, components]))
}

#[cfg(test)]
mod offset_tests {
    use super::offset_of;

    /// Plan SK2 lead decision 4: XGlow's repeated position keys.
    #[test]
    fn a_repeated_position_key_is_an_offset_from_the_first_edge() {
        assert_eq!(
            offset_of("Y", "Bottom.BOTTOM", "-52").as_deref(),
            Some("Bottom.BOTTOM-52")
        );
        assert_eq!(
            offset_of("X", "ButtonDivider.LEFT", "62").as_deref(),
            Some("ButtonDivider.LEFT+62")
        );
        assert_eq!(
            offset_of("Y", "Middle.BOTTOM", "0").as_deref(),
            Some("Middle.BOTTOM+0")
        );
        assert_eq!(offset_of("Y", "12", "0"), None);
        assert_eq!(offset_of("Z", "Bottom.BOTTOM", "1"), None);
    }
}
