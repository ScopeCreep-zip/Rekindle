//! One stylesheet rule per tree node (plan SK2, design §5 "CSS emission"):
//! `[data-skin-node="<id>"]` placed absolutely in its parent's node by its
//! resolved spans, at its `Z`, with its art's piece sets ([`layers`]: caps
//! at their own size, edges and middle tiled from the caps' end edges) for
//! its normal, hover, down and disabled states; an interactive node smaller than 24×24 gets a
//! 24×24 hit area (`::before`, centred, shifted inward at the window's
//! edges as [`hit_area`] lays it; an edge grip's yields to the interactive
//! nodes it overlaps, [`grip_area`], lead decision 6); every node is
//! stacked by its `Z` and then the pack's document order (the shell nests
//! the divs in that order, so a later sibling draws above); a text-sized node is as wide and tall as
//! its text (`max-content`); the views node draws opaque (lead decision 3:
//! the window art may lie under it). Positions never come from the
//! webview; the shell (SK2-S2) only nests `div[data-skin-node]`s.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use rekindle_types::dto::skin::RegionState;
use rekindle_types::dto::skin::WindowLayout;
use rekindle_types::dto::skin_tree::{Anchor, NodePieces, NodeRole, NodeSpan, SkinNode, SkinTree};

use super::pieces::{self, Band};

use super::validate::{grip_area, hit_area, stack_z, text_sized, Stack, HIT};
use crate::regions::css::href;

fn axis(s: NodeSpan, (start, end, len): (&str, &str, &str), text: bool) -> String {
    match s {
        NodeSpan::Fixed {
            at: Anchor::Start(o) | Anchor::Centre { by: o, .. },
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
        // Lead decision 32: the floor `NodeSpan::place` takes, in whole
        // pixels at every parent size (`round(down, …, 1px)` is CSS's
        // floor, as the frame's centred tiles use, `shape/css.rs`).
        NodeSpan::Fixed {
            at: Anchor::Centre { by, before, after },
            size,
        } => {
            let (b, a) = (i32::from(before), i32::from(after));
            let k = b + a - i32::from(size);
            let sign = if k < 0 { '-' } else { '+' };
            format!(
                "{start}: calc({}px + round(down, calc((100% {sign} {}px) / 2), 1px)); {len}: {size}px;",
                i32::from(by) - b,
                k.abs()
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
    /// The interactive nodes that are not grips, with their boxes.
    live: Vec<(&'a str, [i32; 4])>,
    stack: Stack<'a>,
}

/// The `::before` hit area of an interactive node smaller than 24×24, at
/// its offset from the node's box (shifted inward at the window's edges).
fn hit_rule(out: &mut String, sel: &str, n: &SkinNode, ctx: &Ctx<'_>) {
    let small = |s: NodeSpan| size(s).is_some_and(|v| i32::from(v) < HIT);
    let grip = matches!(n.role, NodeRole::Resize(_));
    if !n.role.interactive() || !(grip || small(n.x) || small(n.y)) || text_sized(n) {
        return;
    }
    let Some(area) = ctx.boxes.get(n.id.as_str()) else {
        return;
    };
    let hit = if grip {
        let yields: Vec<_> = ctx
            .live
            .iter()
            .filter(|(id, _)| ctx.stack.yields(&n.id, id))
            .map(|(_, b)| *b)
            .collect();
        grip_area(*area, ctx.window, &yields)
    } else {
        hit_area(*area, ctx.window)
    };
    // A grip's own box takes no pointer events (lead decision 10): only its
    // hit area, which leaves the nodes it yields to, does.
    let events = if grip {
        "\n    pointer-events: auto;"
    } else {
        ""
    };
    let _ = writeln!(
        out,
        "  {sel}::before {{\n    content: \"\";\n    position: absolute;\n    left: {}px;\n    top: {}px;\n    width: {}px;\n    height: {}px;{events}\n  }}",
        hit[0] - area[0],
        hit[1] - area[1],
        hit[2],
        hit[3]
    );
}

/// One state's background layers on the node itself and on its `::after`
/// (the row band, spec A7), as declarations; `None` for an image `images`
/// lacks. The node's own boxes are its border box (the whole box) and, when
/// a side piece needs it, its content box inset by the corner rows (the
/// column band); the `::after` is laid at the row band and inset by the
/// corner rows, so its content box is the middle band. Every layer is laid
/// from its band's start or end ([`pieces::cell`]) and clipped to the band,
/// exactly as [`pieces::draw`] lays it for the outline.
fn layers(
    set: &NodePieces,
    images: &BTreeMap<String, Vec<u8>>,
    base: &str,
) -> Option<(String, Option<String>)> {
    let [left, right, top, bottom] = pieces::insets(set);
    let has = |band: Band| set.keys().any(|s| pieces::cell(*s).band == band);
    let after = (left > 0 || right > 0) && (has(Band::Row) || has(Band::Middle));
    let padded = (top > 0 || bottom > 0)
        && (has(Band::Column) || (has(Band::Middle) && left == 0 && right == 0));
    let (mut own, mut over): (Vec<[String; 4]>, Vec<[String; 4]>) = (Vec::new(), Vec::new());
    for (slot, piece) in set {
        let c = pieces::cell(*slot);
        let h = href(base, &piece.image, images)?;
        let pos = format!(
            "{} {}",
            if c.end.0 { "100%" } else { "0" },
            if c.end.1 { "100%" } else { "0" }
        );
        let rep = match c.repeat {
            (true, true) => "repeat",
            (true, false) => "repeat-x",
            (false, true) => "repeat-y",
            (false, false) => "no-repeat",
        };
        let inner = if padded { "content-box" } else { "border-box" };
        let (list, area) = match c.band {
            Band::Row if after => (&mut over, "padding-box"),
            Band::Middle if after => (&mut over, "content-box"),
            Band::Box | Band::Row => (&mut own, "border-box"),
            Band::Column | Band::Middle => (&mut own, inner),
        };
        list.push([
            format!("url(\"{h}\")"),
            pos,
            rep.to_owned(),
            area.to_owned(),
        ]);
    }
    let decls = |list: &[[String; 4]]| {
        let col = |i: usize| {
            list.iter()
                .map(|x| x[i].as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        format!(
            "background-image: {}; background-position: {}; background-repeat: {}; background-origin: {}; background-clip: {};",
            col(0),
            col(1),
            col(2),
            col(3),
            col(3)
        )
    };
    let mut main = if own.is_empty() {
        "background-image: none;".to_owned()
    } else {
        decls(&own)
    };
    let _ = write!(
        main,
        " box-sizing: border-box; padding: {}px 0 {}px 0;",
        if padded { top } else { 0 },
        if padded { bottom } else { 0 }
    );
    let over = after.then(|| {
        format!(
            "content: \"\"; position: absolute; z-index: -1; pointer-events: none; box-sizing: border-box; left: {left}px; right: {right}px; top: 0; bottom: 0; padding: {top}px 0 {bottom}px 0; {}",
            decls(&over)
        )
    });
    Some((main, over))
}

fn node(out: &mut String, n: &SkinNode, ctx: &Ctx<'_>) -> Option<()> {
    let (root, images, base) = (ctx.root, ctx.images, ctx.base);
    let text = text_sized(n);
    let sel = format!("{root} [data-skin-node=\"{}\"]", n.id);
    let mut decls = format!(
        "position: absolute; {} {} z-index: {};",
        axis(n.x, ("left", "right", "width"), text),
        axis(n.y, ("top", "bottom", "height"), text),
        stack_z(n)
    );
    if n.role == NodeRole::Views {
        decls.push_str(" background-color: var(--color-xfire-bg-dark);");
    }
    if matches!(n.role, NodeRole::Resize(_)) {
        decls.push_str(" pointer-events: none;");
    }
    let mut after = None;
    if let Some(art) = &n.art {
        let (main, over) = layers(&art.normal, images, base)?;
        let _ = write!(decls, " {main}");
        after = over;
    }
    let _ = writeln!(out, "  {sel} {{\n    {decls}\n  }}");
    let normal_after = after.is_some();
    if let Some(over) = after {
        let _ = writeln!(out, "  {sel}::after {{\n    {over}\n  }}");
    }
    for (state, set) in n.art.iter().flat_map(|a| a.states.iter()) {
        let pseudo = match state {
            RegionState::Hover => ":hover",
            RegionState::Down => ":active",
            RegionState::Disabled => "[aria-disabled=\"true\"]",
            _ => continue,
        };
        let (main, over) = layers(set, images, base)?;
        let _ = writeln!(out, "  {sel}{pseudo} {{\n    {main}\n  }}");
        match over {
            Some(over) => {
                let _ = writeln!(out, "  {sel}{pseudo}::after {{\n    {over}\n  }}");
            }
            None if normal_after => {
                let _ = writeln!(
                    out,
                    "  {sel}{pseudo}::after {{\n    background-image: none;\n  }}"
                );
            }
            None => {}
        }
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
    let boxes = tree.boxes(w, h);
    let live = boxes
        .iter()
        .filter(|(n, _)| n.role.interactive() && !matches!(n.role, NodeRole::Resize(_)))
        .map(|(n, b)| (n.id.as_str(), *b))
        .collect();
    let ctx = Ctx {
        root,
        images,
        base,
        boxes: boxes.iter().map(|(n, b)| (n.id.as_str(), *b)).collect(),
        window: (w, h),
        live,
        stack: Stack::new(&tree.root),
    };
    let mut out = String::new();
    for n in &tree.root {
        node(&mut out, n, &ctx)?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use rekindle_types::dto::skin_tree::{NodeSpan, SkinNode};

    use super::{axis, text_sized};
    use crate::{builtin, images, manifest, package};

    /// The start `decls` (one axis's declarations from [`axis`]) gives a
    /// node in a parent `len` px long, as the browser computes it.
    fn start_of(decls: &str, len: i32) -> Option<i32> {
        let px = |v: &str| v.trim().strip_suffix("px")?.parse::<i32>().ok();
        let val = |p: &str| {
            let at = decls.find(&format!("{p}: "))?;
            decls[at + p.len() + 2..].split_once(';').map(|(v, _)| v)
        };
        let Some(v) = val("left") else {
            return Some(len - px(val("right")?)? - px(val("width")?)?);
        };
        if let Some(n) = px(v) {
            return Some(n);
        }
        // A plain half: whole pixels only in an even parent.
        if let Some(n) = v
            .strip_prefix("calc(50% - ")
            .and_then(|r| px(r.strip_suffix(')')?))
        {
            return (len % 2 == 0).then_some(len / 2 - n);
        }
        // calc(Cpx + round(down, calc((100% ± Kpx) / 2), 1px))
        let c = px(v.strip_prefix("calc(")?.split_once('+')?.0)?;
        let (sign, k) = v.split_once("100% ")?.1.split_once(' ')?;
        let k = px(k.split_once(')')?.0)?;
        let k = if sign == "-" { -k } else { k };
        Some(c + (len + k).div_euclid(2))
    }

    fn walk(n: &SkinNode, (w, h): (i32, i32), wrong: &mut Vec<String>, id: &str) {
        let (_, wide) = n.x.place(w);
        let (_, tall) = n.y.place(h);
        if !text_sized(n) {
            for (span, len) in [(n.x, w), (n.y, h)] {
                let css = axis(span, ("left", "right", "width"), false);
                let want = span.place(len).0;
                if start_of(&css, len) != Some(want) {
                    wrong.push(format!("{id} {}: {css} at {len} != {want}", n.id));
                }
            }
        }
        for c in &n.children {
            walk(c, (wide, tall), wrong, id);
        }
    }

    /// Lead decision 32: every built-in node's stylesheet position is
    /// `NodeSpan::place`'s, in whole pixels, at an odd and an even window.
    #[test]
    fn every_builtin_node_sits_where_place_lays_it_at_odd_and_even_sizes() {
        let mut wrong = Vec::new();
        let mut centred = 0;
        for b in builtin::PACKS {
            let id = b.id;
            let package = package::read(b.bytes).unwrap();
            let m = manifest::parse(&package).unwrap();
            images::validate(&package, &m).unwrap();
            for l in m.windows.values() {
                let Some(tree) = &l.tree else { continue };
                let w = i32::from(l.min_width);
                let h = i32::from(l.min_height);
                for size in [(w, h), (w + 1, h + 1)] {
                    for n in &tree.root {
                        walk(n, size, &mut wrong, id);
                    }
                }
                centred += tree
                    .boxes(w, h)
                    .iter()
                    .filter(|(n, _)| {
                        [n.x, n.y].iter().any(|s| {
                            matches!(
                                s,
                                NodeSpan::Fixed {
                                    at: super::Anchor::Centre { .. },
                                    ..
                                }
                            )
                        })
                    })
                    .count();
            }
        }
        assert!(centred > 0, "no centred node to check");
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }
}
