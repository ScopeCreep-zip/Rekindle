//! One stylesheet rule per tree node (plan SK2, design §5 "CSS emission"):
//! `[data-skin-node="<id>"]` placed absolutely in its parent's node by its
//! resolved spans, at its `Z`, with its art's piece sets ([`layers`]: caps
//! at their own size, edges and middle tiled from the caps' end edges) for
//! its normal, hover, down and disabled states; an interactive node smaller than 24×24 is its
//! served target box (plan SK7, [`super::target`]: margins grow the box,
//! a transparent border of the art's inset keeps the art, its children and
//! its text at their own place and size); a resize grip, and a drag area
//! with holes, keeps its art box and takes presses only on its `::before`,
//! laid at its frame band or art box less the press boxes of the controls
//! it would lie over (plan SK11, [`grip_band`], [`super::target::grips`]),
//! so it takes no press over a control (lead decision 6); every node is
//! stacked by its `Z` and then the pack's document order (the shell nests
//! the divs in that order, so a later sibling draws above); a text-sized node is as wide and tall as
//! its text (`max-content`); the views node draws opaque (lead decision 3:
//! the window art may lie under it). Positions never come from the
//! webview; the shell (SK2-S2) only nests `div[data-skin-node]`s.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use rekindle_types::dto::skin::{RegionState, WindowLayout};
use rekindle_types::dto::skin_tree::{NodePieces, NodeRole, NodeTarget, SkinNode, SkinTree};

use super::pieces::{self, Band};
use super::span_css::{axis, target_len};

use super::validate::{stack_z, text_sized};
use crate::regions::css::href;

/// The roles the shell draws as an `<input>` in the node's box: the name
/// while it is edited and the status line.
const fn is_input(role: NodeRole) -> bool {
    matches!(role, NodeRole::NameField | NodeRole::StatusText)
}

/// The state the served refusal copy (`NodeArt::refused`, plan DIS3)
/// draws on: the shell's `data-state="disabled"` for any cause but
/// `windowInactive` (the Host-sent cause of the pack's unfocused-window
/// look, SkinTreeWindow.tsx), so a window-inactive node keeps the pack's
/// own Disabled art, or its Normal art where the pack draws none.
const REFUSED: &str = "[data-state=\"disabled\"]:not([data-state-cause=\"windowInactive\"])";

/// What every node's rule reads: the window's root selector, its package
/// images and base, and each node's box in the smallest window.
struct Ctx<'a> {
    root: &'a str,
    images: &'a BTreeMap<String, Vec<u8>>,
    base: &'a str,
    /// Each grip's box and the holes cut from it ([`super::target::grips`]).
    holes: BTreeMap<String, ([i32; 4], Vec<[i32; 4]>)>,
}

/// One state's background layers on the node itself and on its `::after`
/// (the row band, spec A7), as declarations; `None` for an image `images`
/// lacks. The node's own boxes are its border box (the whole box) and, when
/// a side piece needs it, its content box inset by the corner rows (the
/// column band); the `::after` is laid at the row band and inset by the
/// corner rows, so its content box is the middle band. Every layer is laid
/// from its band's start or end ([`pieces::cell`]) and clipped to the band,
/// exactly as [`pieces::draw`] lays it for the outline. `outer` is the box
/// that is the art's whole box: the border box, or the padding box of a
/// node grown to its target (its border is the art's inset, [`target`]).
fn layers(
    set: &NodePieces,
    images: &BTreeMap<String, Vec<u8>>,
    base: &str,
    outer: &str,
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
        let inner = if padded { "content-box" } else { outer };
        let (list, area) = match c.band {
            Band::Row if after => (&mut over, "padding-box"),
            Band::Middle if after => (&mut over, "content-box"),
            Band::Box | Band::Row => (&mut own, outer),
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
    // Longhands only (plan SK15a): a shorthand here, after the shell's layer,
    // reset the field's served `padding-inline` (`skin-tree.css` `.skin-name`).
    let _ = write!(
        main,
        " box-sizing: border-box; padding-top: {}px; padding-bottom: {}px;",
        if padded { top } else { 0 },
        if padded { bottom } else { 0 }
    );
    let over = after.then(|| {
        format!(
            "content: \"\"; position: absolute; z-index: -1; pointer-events: none; box-sizing: border-box; left: {left}px; right: {right}px; top: 0; bottom: 0; padding-top: {top}px; padding-bottom: {bottom}px; {}",
            decls(&over)
        )
    });
    Some((main, over))
}

/// The declarations that make the node's box its served target box (plan
/// SK7): margins grow it from where its axes lay its art, a fixed axis
/// takes the target's length, and a transparent border of the art's inset
/// keeps the art (drawn on the padding box), its children and its text at
/// their own place and size.
fn target(decls: &mut String, (n, prev): (&SkinNode, &[SkinNode]), t: NodeTarget) {
    let [ax, ay, aw, ah] = t.art;
    let right = t.area[2] - ax - aw;
    let bottom = t.area[3] - ay - ah;
    let _ = write!(
        decls,
        " box-sizing: border-box; margin: {}px {}px {}px {}px; border: solid transparent; border-width: {ay}px {right}px {bottom}px {ax}px;",
        -ay, -right, -bottom, -ax
    );
    for (span, x, prop, i) in [(&n.x, true, "width", 2), (&n.y, false, "height", 3)] {
        if let Some(l) = target_len(span, prev, x, (t.area[i], t.art[i])) {
            let _ = write!(decls, " {prop}: {l};");
        }
    }
}

/// A grip's clip (plan SK11): its box with each hole cut out (even-odd,
/// the holes disjoint), so a press in a hole reaches the listening node
/// under it while the rest of the band, 24 px thick, takes the resize.
fn clip(a: [i32; 4], holes: &[[i32; 4]]) -> String {
    let mut path = format!("M0 0 H{} V{} H0 Z", a[2], a[3]);
    for h in holes {
        let _ = write!(path, " M{} {} h{} v{} h{} Z", h[0], h[1], h[2], h[3], -h[2]);
    }
    format!(" clip-path: path(evenodd, \"{path}\");")
}

/// A grip's pointer area (plan SK11, [`super::target::band`]): the grip's
/// node keeps its art box and takes no press (`pointer-events: none`), and
/// its `::before`, laid at the served band and clipped by its holes, takes
/// the press for it (a press on a pseudo-element reaches its element). The
/// grip's own child nodes take their presses again.
fn grip_band(
    sel: &str,
    grip: &SkinNode,
    target: NodeTarget,
    holes: Option<&([i32; 4], Vec<[i32; 4]>)>,
) -> String {
    let [left, top, width, height] = target.area;
    let cut = holes.map_or(String::new(), |(area, cuts)| clip(*area, cuts));
    let mut rule = format!(
        "  {sel}::before {{\n    content: \"\"; position: absolute; left: {left}px; top: {top}px; width: {width}px; height: {height}px; pointer-events: auto;{cut}\n  }}\n"
    );
    if !grip.children.is_empty() {
        let _ = writeln!(
            rule,
            "  {sel} > [data-skin-node] {{\n    pointer-events: auto;\n  }}"
        );
    }
    rule
}

/// Whether target `t` lies inside its art on some side: a target cut clear
/// of the press area of a listening node drawn over its art (plan SK12,
/// [`super::target`]), which no border can draw, as a border only grows the
/// box.
const fn cut(t: NodeTarget) -> bool {
    let [ax, ay, aw, ah] = t.art;
    ax < 0 || ay < 0 || t.area[2] - ax - aw < 0 || t.area[3] - ay - ah < 0
}

/// A cut press area (plan SK12): the node keeps its art box, which draws in
/// the pack's order and takes no press, and its `::before`, laid at the
/// served area, takes the press for it (as a grip's band, [`grip_band`]),
/// so a press in the two arts' overlap reaches only the node drawn on top
/// there ([`super::target`]'s `under`). Listening nodes under it take their
/// presses again.
fn press_band(sel: &str, t: NodeTarget) -> String {
    let [left, top, width, height] = t.area;
    format!(
        "  {sel}::before {{\n    content: \"\"; position: absolute; left: {left}px; top: {top}px; width: {width}px; height: {height}px; pointer-events: auto;\n  }}\n  {sel} [data-skin-on] {{\n    pointer-events: auto;\n  }}\n"
    )
}

/// A text field's `<input>` laid over its whole served press area (plan
/// SK15a; it filled only the art box, so Alloy's `StatusEdit` took presses
/// in 15 of its 20 px), in the art box's coordinates, with a transparent
/// border keeping its text where the art and the served inset put it.
fn input_area(sel: &str, n: &SkinNode, t: NodeTarget) -> String {
    let [ax, ay, aw, ah] = t.art;
    let [left, top, width, height] = t.area;
    let (il, ir) = n
        .inset
        .map_or((0, 0), |i| (i32::from(i.left), i32::from(i.right)));
    let right = (width - ax - aw + ir).max(0);
    let bottom = (height - ay - ah).max(0);
    format!(
        "  {sel} > input {{\n    position: absolute; left: {left}px; top: {top}px; width: {width}px; height: {height}px; box-sizing: border-box; margin: 0; border: solid transparent; border-width: {}px {right}px {bottom}px {}px; background-clip: padding-box; pointer-events: auto;\n  }}\n",
        ay.max(0),
        (ax + il).max(0)
    )
}

fn node(out: &mut String, (n, prev): (&SkinNode, &[SkinNode]), ctx: &Ctx<'_>) -> Option<()> {
    let (root, images, base) = (ctx.root, ctx.images, ctx.base);
    let text = text_sized(n);
    let sel = format!("{root} [data-skin-node=\"{}\"]", n.id);
    let mut decls = format!(
        "position: absolute; {} {} z-index: {};",
        axis(&n.x, prev, ("left", "right", "width"), text),
        axis(&n.y, prev, ("top", "bottom", "height"), text),
        stack_z(n)
    );
    if n.role == NodeRole::Views {
        decls.push_str(" background-color: var(--color-xfire-bg-dark);");
    }
    let grip = n.frame_node();
    let outer = match n.target {
        Some(t) if !grip && !cut(t) => {
            target(&mut decls, (n, prev), t);
            "padding-box"
        }
        _ => "border-box",
    };
    let band = n.target.and_then(|t| {
        if grip {
            Some(grip_band(&sel, n, t, ctx.holes.get(&n.id)))
        } else {
            cut(t).then(|| press_band(&sel, t))
        }
    });
    if band.is_some() {
        decls.push_str(" pointer-events: none;");
    }
    let mut after = None;
    if let Some(art) = &n.art {
        let (main, over) = layers(&art.normal, images, base, outer)?;
        let _ = write!(decls, " {main}");
        after = over;
    }
    let _ = writeln!(out, "  {sel} {{\n    {decls}\n  }}");
    if let Some(band) = band {
        out.push_str(&band);
    }
    if let Some(t) = n.target.filter(|_| !grip && is_input(n.role)) {
        out.push_str(&input_area(&sel, n, t));
    }
    let normal_after = after.is_some();
    if let Some(over) = after {
        let _ = writeln!(out, "  {sel}::after {{\n    {over}\n  }}");
    }
    let states = n.art.iter().flat_map(|a| {
        a.states
            .iter()
            .filter_map(|(state, set)| {
                let pseudo = match state {
                    RegionState::Hover => ":hover",
                    RegionState::Down => ":active",
                    // The shell's `data-state="disabled"` (SkinTreeWindow.tsx),
                    // set for a refusal and for the pack's unfocused-window look
                    // alike; `aria-disabled` marks only a refusal (CR1, lead
                    // ruling (a)).
                    RegionState::Disabled => "[data-state=\"disabled\"]",
                    _ => return None,
                };
                Some((pseudo, set))
            })
            .chain(a.refused.as_ref().map(|set| (REFUSED, set)))
    });
    for (pseudo, set) in states {
        let (main, over) = layers(set, images, base, outer)?;
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
    for (i, c) in n.children.iter().enumerate() {
        node(out, (c, &n.children[..i]), ctx)?;
    }
    Some(())
}

/// The rules of `tree` under `root` (the window's `.app-frame` selector),
/// laid from its spans and served targets, a grip's holes measured in
/// `layout`'s window at its served minimum; `None` for an art image `images` lacks
/// (never for a validated package).
#[must_use]
pub fn tree_css(
    root: &str,
    tree: &SkinTree,
    layout: &WindowLayout,
    images: &BTreeMap<String, Vec<u8>>,
    base: &str,
) -> Option<String> {
    let window = super::target::window(layout);
    let holes = super::target::grip_holes(tree, window)
        .into_iter()
        .map(|(id, a, h)| (id, (a, h)))
        .collect();
    let ctx = Ctx {
        root,
        images,
        base,
        holes,
    };
    // The shell's tree container (`.skin-tree`) is the pack's window: the
    // whole window less the outsets every root node is laid in.
    let to = tree.outsets;
    let mut out = format!(
        "  {root} > .skin-tree {{\n    display: block; position: absolute; top: {}px; right: {}px; bottom: {}px; left: {}px;\n  }}\n",
        to.top, to.right, to.bottom, to.left
    );
    for (i, n) in tree.root.iter().enumerate() {
        node(&mut out, (n, &tree.root[..i]), &ctx)?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use rekindle_types::dto::skin::SkinWindow;
    use rekindle_types::dto::skin_tree::{NodeSpan, SkinNode, SkinTree};

    use super::super::test_support::{css_len, Ratio};
    use super::{axis, text_sized};
    use crate::{builtin, images, manifest, package};

    /// The start `decls` (one axis's declarations from [`axis`]) gives a
    /// node in a parent `len` px long, as the browser computes it.
    /// The start `decls` puts a box at in a parent `len` px long: its
    /// `left`, or `len - right - width`, each evaluated as the browser
    /// evaluates the emitted `calc(...)`/`round(down, …, 1px)`; `None`
    /// when the start is not a whole pixel (a plain half in an odd parent).
    fn start_of(decls: &str, len: i32) -> Option<i32> {
        let val = |p: &str| {
            let at = decls.find(&format!("{p}: "))?;
            decls[at + p.len() + 2..].split_once(';').map(|(v, _)| v)
        };
        let start = match val("left") {
            Some(v) => css_len(v, len)?,
            None => css_len(val("right")?, len)?
                .add(css_len(val("width")?, len)?)
                .neg()
                .add(Ratio::whole(i64::from(len))),
        };
        start.px()
    }

    fn walk(n: &SkinNode, (w, h): (i32, i32), wrong: &mut Vec<String>, id: &str) {
        let (_, wide) = n.x.place(w);
        let (_, tall) = n.y.place(h);
        if !text_sized(n) {
            for (span, len) in [(&n.x, w), (&n.y, h)] {
                let css = axis(span, &[], ("left", "right", "width"), false);
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
                        [n.x.clone(), n.y.clone()].iter().any(|s| {
                            matches!(
                                s,
                                NodeSpan::Fixed {
                                    at: rekindle_types::dto::skin_tree::Anchor::Centre { .. },
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

    /// A built-in's served main tree and the stylesheet the Host serves it.
    fn served(id: &str) -> (SkinTree, String) {
        let (_, b) = builtin::find(&format!("{}{id}", builtin::PREFIX)).unwrap();
        let p = package::read(b.bytes).unwrap();
        let (mut m, made) = manifest::served_made(&p).unwrap();
        let mut files = images::validate(&p, &m).unwrap();
        files.extend(made);
        let l = m.windows.remove(&SkinWindow::Main).unwrap();
        let tree = l.tree.clone().unwrap();
        let css = super::tree_css(".f", &tree, &l, &files, "x").unwrap();
        (tree, css)
    }

    fn all<'a>(nodes: &'a [SkinNode], out: &mut Vec<&'a SkinNode>) {
        for n in nodes {
            out.push(n);
            all(&n.children, out);
        }
    }

    fn sel(id: &str) -> String {
        format!(".f [data-skin-node=\"{id}\"]")
    }

    /// The declarations of each rule laid on the node `sel` itself (its
    /// states and pseudo-elements, never a descendant's rule).
    fn own<'a>(css: &'a str, sel: &str) -> Vec<&'a str> {
        let lines: Vec<&str> = css.lines().map(str::trim).collect();
        lines
            .windows(2)
            .filter(|w| {
                w[0].strip_prefix(sel)
                    .and_then(|r| r.strip_suffix('{'))
                    .is_some_and(|r| r == " " || !r.starts_with(' '))
            })
            .map(|w| w[1])
            .collect()
    }

    /// Plan SK15a: no served rule of a text field with an inset writes the
    /// `padding` shorthand or `padding-inline`, so the shell's
    /// `padding-inline: var(--field-inset-…)` holds (Alloy's name clear of
    /// its status edit's arrow, Simple-X's of its `MinimizeBox`).
    #[test]
    fn a_field_with_an_inset_keeps_the_shells_inline_padding() {
        let mut fields = 0;
        for b in builtin::PACKS {
            let (tree, css) = served(b.id);
            let mut nodes = Vec::new();
            all(&tree.root, &mut nodes);
            for n in nodes.into_iter().filter(|n| n.inset.is_some()) {
                let rules = own(&css, &sel(&n.id));
                assert!(!rules.is_empty(), "{} {}: no rule", b.id, n.id);
                for r in rules {
                    assert!(
                        !r.contains("padding:") && !r.contains("padding-inline"),
                        "{} {}: {r}",
                        b.id,
                        n.id
                    );
                }
                fields += 1;
            }
        }
        assert!(fields > 0, "no field with an inset");
        for pack in ["alloy", "simple-x"] {
            let (_, css) = served(pack);
            let rule = own(&css, &sel("UserNameField"))[0];
            assert!(
                rule.contains("padding-top: 0px; padding-bottom: 0px;")
                    && !rule.contains("padding:"),
                "{pack} UserNameField: {rule}"
            );
        }
    }

    /// Plan SK15a: a text field drawn as an input (the name while edited,
    /// the status line) with a served target lays its input over that whole
    /// area, so a press anywhere the Host claims focuses it.
    #[test]
    fn an_input_field_takes_its_whole_press_area() {
        let mut inputs = 0;
        for b in builtin::PACKS {
            let (tree, css) = served(b.id);
            let mut nodes = Vec::new();
            all(&tree.root, &mut nodes);
            for n in nodes.into_iter().filter(|n| super::is_input(n.role)) {
                let Some(target) = n.target else { continue };
                let [left, top, width, height] = target.area;
                let head = format!("{} > input {{", sel(&n.id));
                let rule = css
                    .lines()
                    .map(str::trim)
                    .skip_while(|l| *l != head)
                    .nth(1)
                    .unwrap_or_else(|| panic!("{} {}: no input rule", b.id, n.id));
                let area =
                    format!("left: {left}px; top: {top}px; width: {width}px; height: {height}px;");
                assert!(rule.contains(&area), "{} {}: {rule}", b.id, n.id);
                inputs += 1;
            }
        }
        assert!(inputs > 0, "no input field with a target");
        let (_, css) = served("alloy");
        let head = format!("{} > input {{", sel("StatusEdit"));
        let rule = css.lines().map(str::trim).skip_while(|l| *l != head).nth(1);
        assert_eq!(
            rule,
            Some("position: absolute; left: 0px; top: -4px; width: 165px; height: 20px; box-sizing: border-box; margin: 0; border: solid transparent; border-width: 4px 0px 1px 0px; background-clip: padding-box; pointer-events: auto;")
        );
    }

    /// Plan CR1, lead ruling (a): a pack's Disabled art draws on the shell's
    /// `data-state="disabled"`, which a window-inactive Disabled sets without
    /// `aria-disabled`, so the unfocused-window look shows the pack's art.
    #[test]
    fn disabled_art_draws_on_the_shells_data_state() {
        let (_, css) = served("xslimfire");
        let head = format!("{}[data-state=\"disabled\"] {{", sel("ChatButton"));
        assert!(
            css.lines().any(|l| l.trim() == head),
            "xslimfire ChatButton: no {head}"
        );
        for b in builtin::PACKS {
            let (_, css) = served(b.id);
            assert!(
                !css.contains("aria-disabled"),
                "{}: art keyed on aria-disabled",
                b.id
            );
        }
    }
}
