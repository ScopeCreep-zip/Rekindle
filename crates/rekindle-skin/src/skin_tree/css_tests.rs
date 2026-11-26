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
    let mut m = manifest::served(&p).unwrap();
    let files = images::validate(&p, &m).unwrap();
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
            rule.contains("padding-top: 0px; padding-bottom: 0px;") && !rule.contains("padding:"),
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
