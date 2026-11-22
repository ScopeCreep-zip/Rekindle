//! The popups a window's tiles launch (plan SK2 P1, design §2.3: "`<Popup>`
//! children of `LaunchSkin` become `windows.<popup>.tree` entries"). Each
//! popup tile of the pack's `<Popups>` that a node's `LaunchPopup` names is
//! laid by the same walk as the window's own tree, in its own box: its
//! component's `DefaultWidth` (else `MinWidth`) wide, its `X` resolved
//! against the launching window's top-level tiles or the window itself
//! (Symbiosis's `MainInfoViewPopup` `X="MainWindow.Right-13"`,
//! `Popups.xml:10`; Classic's `X="MainRightOuterEdge.RIGHT"`,
//! `xfirepopups.xml:2`), and its `Y` with the component's `ResizeY`
//! stretched in the window's height (`IndentTop="23" IndentBottom="23"`).
//! A `LaunchPopup` naming a tile the pack's `<Popups>` lacks refuses the
//! pack, like an unknown grip.

use std::collections::BTreeSet;

use rekindle_types::dto::skin::{
    Insets, PopupAnchor, PopupEdge, PopupLayout, SkinError, SkinRule, WindowLayout,
};
use rekindle_types::dto::skin_action::SkinAction;
use rekindle_types::dto::skin_tree::{Anchor, NodeSpan, SkinNode, SkinTree};

use super::anchor::{axis, Sibs, Sizes, X, Y};
use super::clip::{clip, Art, Images};
use super::import::{bad, limit_of, Files, Walk};
use super::passes::{pages, qualify};
use crate::import::tree::Node;
use crate::import::ImportError;

/// Every popup name a node of `nodes` launches.
fn launched(nodes: &[SkinNode], out: &mut BTreeSet<String>) {
    for n in nodes {
        for t in &n.triggers {
            if let SkinAction::LaunchPopup(p) = &t.run {
                out.insert(p.popup.clone());
            }
        }
        launched(&n.children, out);
    }
}

/// `span` (laid in the pack's box, the window less `before`/`after`) in
/// the whole window's coordinates.
fn in_window(span: NodeSpan, before: u16, after: u16) -> NodeSpan {
    let (b, a) = (before.cast_signed(), after.cast_signed());
    match span {
        NodeSpan::Fixed {
            at: Anchor::Start(o),
            size,
        } => NodeSpan::Fixed {
            at: Anchor::Start(o.saturating_add(b)),
            size,
        },
        NodeSpan::Fixed {
            at: Anchor::End(o),
            size,
        } => NodeSpan::Fixed {
            at: Anchor::End(o.saturating_add(a)),
            size,
        },
        NodeSpan::Fixed { at, size } => NodeSpan::Fixed { at, size },
        NodeSpan::Stretch { start, end } => NodeSpan::Stretch {
            start: start.saturating_add(b),
            end: end.saturating_add(a),
        },
    }
}

/// The popup's left side from its `X` span in window coordinates.
fn anchor_of(name: &str, x: NodeSpan) -> Result<PopupAnchor, ImportError> {
    match x {
        NodeSpan::Fixed {
            at: Anchor::Start(o),
            ..
        } => Ok(PopupAnchor {
            edge: PopupEdge::Left,
            offset: o,
        }),
        NodeSpan::Fixed {
            at: Anchor::End(o),
            size,
        } => Ok(PopupAnchor {
            edge: PopupEdge::Right,
            offset: o
                .checked_add(size.cast_signed())
                .and_then(i16::checked_neg)
                .ok_or_else(|| bad(format!("{name}: X out of range")))?,
        }),
        _ => Err(bad(format!("{name}: a popup's X is a fixed edge"))),
    }
}

/// Lays popup tile `tile` beside the window `parent` walked.
fn lay(
    parent: &Walk<'_>,
    (tile, window_name): (&Node, &str),
    (outsets, sibs): (Insets, &Sibs),
    built: &mut (WindowLayout, Files),
) -> Result<(String, PopupLayout), ImportError> {
    let name = tile.attr_ci("Name").unwrap_or_default().trim().to_owned();
    let comp = tile
        .attr_ci("Component")
        .and_then(|c| parent.by_name.get(&c.to_ascii_lowercase()).copied());
    let min_width = limit_of(comp, "MinWidth").unwrap_or(0);
    let width = limit_of(comp, "DefaultWidth").unwrap_or(min_width);
    if width == 0 {
        return Err(bad(format!("{name}: a popup with no width")));
    }
    let min_height = limit_of(comp, "MinHeight").unwrap_or(0);
    let mut placed = tile.clone();
    if let (None, Some(v)) = (
        tile.attr_ci("ResizeY"),
        comp.and_then(|c| c.attr_ci("ResizeY")),
    ) {
        placed.attrs.insert("ResizeY".to_owned(), v.to_owned());
    }
    let mut refs = sibs.clone();
    let whole = (
        NodeSpan::Stretch {
            start: -outsets.left.cast_signed(),
            end: -outsets.right.cast_signed(),
        },
        NodeSpan::Stretch {
            start: -outsets.top.cast_signed(),
            end: -outsets.bottom.cast_signed(),
        },
    );
    refs.insert(window_name.to_ascii_lowercase(), whole);
    let both = (&refs, &Sizes::new());
    let x = in_window(
        axis(&placed, &X, width, None, both)?,
        outsets.left,
        outsets.right,
    );
    let y = in_window(
        axis(&placed, &Y, min_height, None, both)?,
        outsets.top,
        outsets.bottom,
    );
    let mut walk = Walk {
        window: parent.window,
        by_name: parent.by_name.clone(),
        piece: parent.piece,
        files: Files::new(),
        images: Images::new(),
        groups: Vec::new(),
        popups: parent.popups.clone(),
        group_ids: parent.group_ids.clone(),
        prefix: format!("{}-", name.to_ascii_lowercase()),
    };
    let mut tree = SkinTree {
        outsets: Insets {
            top: 0,
            right: 0,
            bottom: 0,
            left: 0,
        },
        root: walk.walk(tile, None, &Sibs::new(), &[])?,
    };
    pages(&mut tree.root, &walk.groups);
    qualify(&mut tree.root);
    let mut art = Art {
        window: walk.window,
        images: &mut walk.images,
        files: &mut walk.files,
    };
    clip(&mut tree.root, (None, None), &mut art)?;
    built.1.extend(walk.files);
    let layout = PopupLayout {
        anchor: anchor_of(&name, x)?,
        width,
        min_width,
        min_height,
        y,
        tree,
    };
    Ok((name, layout))
}

/// Adds to `built` every popup a node of `tree` launches, each laid from
/// the pack's `<Popups>` tile of that name.
///
/// # Errors
/// A launched popup the pack's `<Popups>` lacks, or what laying it returns.
pub(super) fn add(
    parent: &Walk<'_>,
    (layout, components): (&Node, &[Node]),
    tree: &SkinTree,
    built: &mut (WindowLayout, Files),
) -> Result<(), ImportError> {
    let mut names = BTreeSet::new();
    launched(&tree.root, &mut names);
    let sibs: Sibs = tree
        .root
        .iter()
        .map(|n| (n.id.to_ascii_lowercase(), (n.x, n.y)))
        .collect();
    let window_name = layout.attr_ci("Component").unwrap_or("MainWindow");
    for name in names {
        let tile = popup_tile(components, &name).ok_or_else(|| {
            bad(format!(
                "{name}: launched, not a tile of the pack's <Popups>"
            ))
        })?;
        let (key, popup) = lay(parent, (tile, window_name), (tree.outsets, &sibs), built)?;
        built.0.popups.insert(key, popup);
    }
    Ok(())
}

/// The `<Popups>` tile named `name` among the pack's `components`.
fn popup_tile<'a>(components: &'a [Node], name: &str) -> Option<&'a Node> {
    components
        .iter()
        .filter(|n| n.name.eq_ignore_ascii_case("Popups"))
        .flat_map(|p| p.children_named("Tile"))
        .find(|t| {
            t.attr_ci("Name")
                .is_some_and(|n| n.trim().eq_ignore_ascii_case(name))
        })
}

/// The frame a popup's tree is laid and checked in: its own box at its
/// smallest, no frame tiles.
#[must_use]
pub fn frame(popup: &PopupLayout) -> WindowLayout {
    let none = Insets {
        top: 0,
        right: 0,
        bottom: 0,
        left: 0,
    };
    WindowLayout {
        outsets: none,
        content: none,
        min_width: popup.width,
        min_height: popup.min_height,
        tiles: Vec::new(),
        title: None,
        tree: None,
        popups: std::collections::BTreeMap::new(),
    }
}

/// Checks `layout`'s popups: each tree by the window tree's rules, and
/// every `LaunchPopup` of the window's tree naming one of them.
///
/// # Errors
/// The first popup tree rule broken, or a launch of a popup `layout` lacks.
pub fn check(
    field: &str,
    layout: &WindowLayout,
    images: &std::collections::BTreeMap<String, Vec<u8>>,
) -> Result<(), SkinError> {
    let mut names = BTreeSet::new();
    if let Some(tree) = &layout.tree {
        launched(&tree.root, &mut names);
    }
    if let Some(missing) = names.iter().find(|n| !layout.popups.contains_key(*n)) {
        let at = format!("{field}.popups.{missing}");
        return Err(SkinError::new(
            &at,
            SkinRule::BadValue {
                field: at.clone(),
                expected: "a popup the window's LaunchPopup names".to_owned(),
            },
        ));
    }
    for (name, popup) in &layout.popups {
        let at = format!("{field}.popups.{name}.tree");
        super::validate::check(&at, &popup.tree, &frame(popup), images)?;
    }
    Ok(())
}
