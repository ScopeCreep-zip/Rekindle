//! One stylesheet rule per tree node (plan SK2, design §5 "CSS emission"):
//! `[data-skin-node="<id>"]` placed absolutely in its parent's node by its
//! resolved spans, at its `Z`, with its art at its own size and its hover,
//! down and disabled art; an interactive node smaller than 24×24 gets a
//! 24×24 hit area (`::before`, centred, shifted inward at the window's
//! edges as [`hit_area`] lays it); a text-sized node is as wide and tall as
//! its text (`max-content`); the views node draws opaque (lead decision 3:
//! the window art may lie under it). Positions never come from the
//! webview; the shell (SK2-S2) only nests `div[data-skin-node]`s.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use rekindle_types::dto::skin::RegionState;
use rekindle_types::dto::skin::WindowLayout;
use rekindle_types::dto::skin_tree::{Anchor, NodeRole, NodeSpan, SkinNode, SkinTree};

use super::validate::{hit_area, text_sized, HIT};
use crate::regions::css::href;

fn axis(s: NodeSpan, (start, end, len): (&str, &str, &str), text: bool) -> String {
    match s {
        NodeSpan::Fixed {
            at: Anchor::Start(o) | Anchor::Centre(o),
            size: 0,
        } if text => format!("{start}: {o}px; {len}: max-content;"),
        NodeSpan::Fixed {
            at: Anchor::End(o),
            size: 0,
        } if text => format!("{end}: {o}px; {len}: max-content;"),
        NodeSpan::Fixed {
            at: Anchor::Start(o),
            size,
        } => format!("{start}: {o}px; {len}: {size}px;"),
        NodeSpan::Fixed {
            at: Anchor::End(o),
            size,
        } => format!("{end}: {o}px; {len}: {size}px;"),
        NodeSpan::Fixed {
            at: Anchor::Centre(o),
            size,
        } => {
            format!(
                "{start}: calc(50% - {}px); {len}: {size}px;",
                i32::from(size) / 2 - i32::from(o)
            )
        }
        NodeSpan::Stretch { start: s, end: e } => format!("{start}: {s}px; {end}: {e}px;"),
    }
}

fn size(s: NodeSpan) -> Option<u16> {
    match s {
        NodeSpan::Fixed { size, .. } => Some(size),
        NodeSpan::Stretch { .. } => None,
    }
}

/// What every node's rule reads: the window's root selector, its package
/// images and base, and each node's box in the smallest window.
struct Ctx<'a> {
    root: &'a str,
    images: &'a BTreeMap<String, Vec<u8>>,
    base: &'a str,
    boxes: BTreeMap<&'a str, [i32; 4]>,
    window: (i32, i32),
}

/// The `::before` hit area of an interactive node smaller than 24×24, at
/// its offset from the node's box (shifted inward at the window's edges).
fn hit_rule(out: &mut String, sel: &str, n: &SkinNode, ctx: &Ctx<'_>) {
    let small = |s: NodeSpan| size(s).is_some_and(|v| i32::from(v) < HIT);
    if !n.role.interactive() || !(small(n.x) || small(n.y)) || text_sized(n) {
        return;
    }
    let Some(area) = ctx.boxes.get(n.id.as_str()) else {
        return;
    };
    let hit = hit_area(*area, ctx.window);
    let _ = writeln!(
        out,
        "  {sel}::before {{\n    content: \"\";\n    position: absolute;\n    left: {}px;\n    top: {}px;\n    width: {}px;\n    height: {}px;\n  }}",
        hit[0] - area[0],
        hit[1] - area[1],
        hit[2],
        hit[3]
    );
}

fn node(out: &mut String, n: &SkinNode, ctx: &Ctx<'_>) -> Option<()> {
    let (root, images, base) = (ctx.root, ctx.images, ctx.base);
    let text = text_sized(n);
    let sel = format!("{root} [data-skin-node=\"{}\"]", n.id);
    let mut decls = format!(
        "position: absolute; {} {} z-index: {};",
        axis(n.x, ("left", "right", "width"), text),
        axis(n.y, ("top", "bottom", "height"), text),
        n.z
    );
    if n.role == NodeRole::Views {
        decls.push_str(" background-color: var(--color-xfire-bg-dark);");
    }
    if let Some(art) = &n.art {
        let h = href(base, &art.image, images)?;
        let _ = write!(decls, " background: url(\"{h}\") 0 0 no-repeat;");
    }
    let _ = writeln!(out, "  {sel} {{\n    {decls}\n  }}");
    for (state, path) in n.art.iter().flat_map(|a| a.states.iter()) {
        let pseudo = match state {
            RegionState::Hover => ":hover",
            RegionState::Down => ":active",
            RegionState::Disabled => "[aria-disabled=\"true\"]",
            _ => continue,
        };
        let h = href(base, path, images)?;
        let _ = writeln!(
            out,
            "  {sel}{pseudo} {{\n    background-image: url(\"{h}\");\n  }}"
        );
    }
    hit_rule(out, &sel, n, ctx);
    for c in &n.children {
        node(out, c, ctx)?;
    }
    Some(())
}

/// The rules of `tree` under `root` (the window's `.app-frame` selector),
/// laid in `layout`'s window; `None` for an art image `images` lacks
/// (never for a validated package).
#[must_use]
pub fn tree_css(
    root: &str,
    tree: &SkinTree,
    layout: &WindowLayout,
    images: &BTreeMap<String, Vec<u8>>,
    base: &str,
) -> Option<String> {
    let o = layout.outsets;
    let w = i32::from(layout.min_width) + i32::from(o.left) + i32::from(o.right);
    let h = i32::from(layout.min_height) + i32::from(o.top) + i32::from(o.bottom);
    let ctx = Ctx {
        root,
        images,
        base,
        boxes: tree
            .boxes(w, h)
            .into_iter()
            .map(|(n, b)| (n.id.as_str(), b))
            .collect(),
        window: (w, h),
    };
    let mut out = String::new();
    for n in &tree.root {
        node(&mut out, n, &ctx)?;
    }
    Some(out)
}
