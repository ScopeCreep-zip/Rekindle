//! Plan SK8: every node that draws text is served a usable box on x
//! ([`super::text_box`]), measured on the eleven built-ins at each window's
//! served minimum.

use rekindle_types::dto::skin_tree::{NodeRole, SkinNode, SkinTree};

use super::golden_tests::main_layout;
use super::titles::{laid_window, place, popup_window};

const PACKS: [&str; 11] = [
    "alloy",
    "combat",
    "onyx",
    "simple-x",
    "spacecowboy",
    "stealthx",
    "symbiosis",
    "total-chaos",
    "xfire-classic",
    "xglow",
    "xslimfire",
];

/// A text node laid at its window's minimum: id, its box, its parent's.
struct Laid {
    id: String,
    role: NodeRole,
    words: Option<String>,
    b: [i32; 4],
    parent: [i32; 4],
}

fn texts(nodes: &[SkinNode], parent: [i32; 4], out: &mut Vec<Laid>) {
    for n in nodes {
        let b = place(n, parent);
        let field = matches!(
            n.role,
            NodeRole::NameField | NodeRole::StatusText | NodeRole::RoomName
        );
        let words = n
            .label
            .as_ref()
            .filter(|l| l.glyph.is_none())
            .map(|l| l.text.clone());
        if n.art.is_none() && (field || words.is_some()) {
            out.push(Laid {
                id: n.id.clone(),
                role: n.role,
                words,
                b,
                parent,
            });
        }
        texts(&n.children, b, out);
    }
}

/// Every text node of the pack's main window and its popups.
fn laid(id: &str) -> Vec<Laid> {
    let main = main_layout(id);
    let mut trees: Vec<(&SkinTree, (i32, i32))> = Vec::new();
    if let Some(t) = main.tree.as_ref() {
        trees.push((t, super::target::window(&main)));
    }
    for p in main.popups.values() {
        trees.push((&p.tree, popup_window(p, main.min_height)));
    }
    let mut out = Vec::new();
    for (t, at) in trees {
        texts(&t.root, laid_window(t.outsets, at), &mut out);
    }
    out
}

fn node(id: &str, node: &str) -> Laid {
    laid(id)
        .into_iter()
        .find(|l| l.id == node)
        .unwrap_or_else(|| panic!("{id}: no text node {node}"))
}

use super::content::word_width;

/// Finding 1: Combat's name field `Title` (x at 55, size 0) and Onyx's
/// `MainTitleBar/Title` (x at 20, size 0) are served a positive width
/// that ends at the first press area or text box on their line (plan SK12:
/// Combat's at x 269, the `DownArrow` button's target, as the real app
/// lays it; Onyx's at its parent's end: plan SK15a serves it its 15 px line
/// box, y 6-21, which only touches its `DownArrow`'s target, from y 21), so
/// a long name ellipsizes inside it and runs under no button.
#[test]
fn zero_width_title_anchors_get_their_usable_width() {
    for (pack, start, end) in [("combat", 55, 269), ("onyx", 25, 185)] {
        let t = node(pack, "Title");
        assert_eq!(t.b[0], start, "{pack}: the anchor stays");
        assert!(t.b[2] > 0, "{pack}: Title {:?} cannot ellipsize", t.b);
        assert_eq!(
            t.b[0] + t.b[2],
            end,
            "{pack}: Title {:?} in {:?}",
            t.b,
            t.parent
        );
    }
}

/// Finding 2, plan SK15b: Onyx's menu titles (`Class=Text
/// Stretch=ToContent`, `components.xml:268-273`) are as wide as the words
/// the Host serves plus 1 px, and each next title starts 10 px past that
/// (`X="XfireMenuText.RIGHT+10"`, `mainwindow.xml:42`); each box holds
/// its word.
#[test]
fn onyxs_menu_titles_take_the_room_up_to_the_next_title() {
    let titles = ["XfireMenuText", "ToolsMenuText", "HelpMenuText"].map(|t| node("onyx", t));
    for pair in titles.windows(2) {
        let words = pair[0].words.as_deref().unwrap();
        let end = pair[0].b[0] + word_width(words) + 1;
        assert_eq!(
            pair[1].b[0],
            end + 10,
            "onyx {} after {words:?}",
            pair[1].id
        );
    }
    for t in &titles {
        let words = t.words.as_deref().unwrap();
        assert!(
            t.b[2] >= word_width(words),
            "onyx {} {:?} cuts {words:?}",
            t.id,
            t.b
        );
    }
}

/// Every served label's box holds its word at the 10 px floor wherever the
/// pack leaves room; the labels listed have no room at the pack's places
/// (pack-layout findings for the lead), each at its served width.
#[test]
fn labels_hold_their_words_where_room_exists() {
    let no_room: &[(&str, &str, i32)] = &[
        // Plan SK15b: Simple-X's menu row (`MenuBar` X 14) meets its
        // `TabHeader` (X 125, the same Y 100, `SimpleX.ini:106-107`):
        // Xfire's "File" fit; "Rekindle" (46 px) carries `HelpMenuText`
        // to x 110-134, into `TabNormalText`'s press from x 125, and each
        // stops at the other's press (the Simple-X File glyph rule of
        // design SK15b clears the row).
        ("simple-x", "HelpMenuText", 13),
        ("simple-x", "TabNormalText", 37),
        // Plan SK13: the text starts at x 132, inside `TabServerText`'s
        // press (its `TabServerButton` reaches x 135), and stops there.
        ("xfire-classic", "TabDownloadText", 48),
    ];
    let mut short = Vec::new();
    for pack in PACKS {
        for l in laid(pack) {
            if let Some(w) = l.words.as_deref() {
                if l.b[2] < word_width(w) {
                    short.push((pack, l.id, l.b[2], w.to_owned(), word_width(w)));
                }
            }
        }
    }
    let got: Vec<(&str, &str, i32)> = short.iter().map(|s| (s.0, s.1.as_str(), s.2)).collect();
    assert_eq!(got, no_room, "labels short of their word: {short:?}");
}

/// Plan SK15a: no served text box is shorter than the shell's line box for
/// its text (15 px for the name, 14 px for the rest), so the text pass and
/// the overlap invariant measure the line the shell draws.
#[test]
fn every_text_box_is_as_tall_as_its_line() {
    let mut short = Vec::new();
    for pack in PACKS {
        for l in laid(pack) {
            if l.b[3] < i32::from(super::text_box::line_box(l.role)) {
                short.push((pack, l.id, l.b));
            }
        }
    }
    assert!(
        short.is_empty(),
        "text boxes shorter than their line: {short:?}"
    );
}

/// No served text box reaches past its parent's box on x.
#[test]
fn no_text_box_exceeds_its_parent() {
    let mut past = Vec::new();
    for pack in PACKS {
        for l in laid(pack) {
            let grew = l.b[2] > 0;
            if grew && !(l.b[0] >= l.parent[0] && l.b[0] + l.b[2] <= l.parent[0] + l.parent[2]) {
                past.push((pack, l.id, l.b, l.parent));
            }
        }
    }
    assert!(past.is_empty(), "text boxes past their parent: {past:?}");
}

/// The box of `node` in the pack's main tree laid at its minimum.
fn art_box(pack: &str, node: &str) -> [i32; 4] {
    let main = main_layout(pack);
    let t = main.tree.as_ref().unwrap();
    let o = t.outsets;
    let w = i32::from(main.min_width) + i32::from(o.left) + i32::from(o.right);
    let h = i32::from(main.min_height) + i32::from(o.top) + i32::from(o.bottom);
    t.boxes(w, h)
        .into_iter()
        .find_map(|(n, b)| (n.id == node).then_some(b))
        .unwrap_or_else(|| panic!("{pack}: no node {node}"))
}

/// A grown text node's button art keeps the pack's box: Classic's
/// `TabNormalButton` (`Stretch` -4/-18 around its 40 px `TabNormalText`)
/// stays at x 18-79, Combat's `HelpMenu` part at x 148-183 (its chained place past the
/// served words, plan SK15b), while their
/// text boxes grow.
#[test]
fn a_grown_text_keeps_its_buttons_art_box() {
    for (pack, text, art, want) in [
        (
            "xfire-classic",
            "TabNormalText",
            "TabNormalButton",
            [18, 146, 62, 16],
        ),
        // Plan SK15b: past the served "Rekindle" and "Tools" on the
        // pack's chains, 4 px wider for "Help".
        ("combat", "HelpMenuText", "HelpMenu", [148, 171, 36, 22]),
    ] {
        assert!(node(pack, text).b[2] > 40, "{pack} {text} did not grow");
        assert_eq!(art_box(pack, art), want, "{pack} {art}");
    }
}

/// Plan SK9: a name field beside an 18 px button grows only to the
/// button's 24 px target box (WCAG 2.2 SC 2.5.8 takes precedence over the
/// field's ellipsis room), and the button's served target then meets the
/// field's box without overlapping it.
#[test]
fn a_name_field_stops_at_its_buttons_target_box() {
    use rekindle_types::dto::skin_action::{SkinAction, SkinEvent};

    use super::test_support::Z;
    use super::tests::{acts, fixed, layout, n};

    let field = n("Name", NodeRole::NameField, fixed(10, 0), fixed(10, 16));
    let button = acts(
        n("Min", NodeRole::Button, fixed(100, 18), fixed(10, 16)),
        SkinEvent::Click,
        SkinAction::Minimize,
    );
    let mut l = layout(Some(SkinTree {
        outsets: Z,
        root: vec![field, button],
    }));
    super::text_box::lay(&mut l);
    super::target::nodes(&mut l);
    let tree = l.tree.as_ref().unwrap();
    let field = place(&tree.root[0], laid_window(Z, super::target::window(&l)));
    let t = tree.root[1]
        .target
        .expect("the 18 px button serves a target");
    // The button's art at x 100, its 24 px target from x 97.
    assert_eq!(t.area[0], -3, "button target {t:?}");
    assert_eq!(field[0] + field[2], 97, "name field box {field:?}");
}

/// Plan SK11: the text-box pass lays the tree in the window the
/// stylesheet and the targets lay it in, the served minimum with the
/// layout's outsets (`shape::min_window`), never the bare minimum: a name
/// field at x 10 stops exactly at the 24 px target box of an 18 px button
/// anchored 10 px from the window's end, with a 13 px right overhang
/// (Symbiosis's), as the targets lay that button.
#[test]
fn a_text_box_is_laid_at_the_minimum_with_the_outsets() {
    use rekindle_types::dto::skin_action::{SkinAction, SkinEvent};
    use rekindle_types::dto::skin_tree::{Anchor, NodeSpan};

    use super::tests::{acts, fixed, layout, n};

    let field = n("Name", NodeRole::NameField, fixed(10, 0), fixed(10, 16));
    let end = NodeSpan::Fixed {
        at: Anchor::End(10),
        size: 18,
    };
    let button = acts(
        n("Min", NodeRole::Button, end, fixed(10, 16)),
        SkinEvent::Click,
        SkinAction::Minimize,
    );
    let mut l = layout(Some(SkinTree {
        outsets: super::test_support::Z,
        root: vec![field, button],
    }));
    l.outsets.right = 13;
    let served = crate::shape::min_window(&l).unwrap();
    assert_eq!(
        super::target::window(&l),
        (served.0.into(), served.1.into())
    );
    super::text_box::lay(&mut l);
    let tree = l.tree.as_ref().unwrap();
    let field = place(
        &tree.root[0],
        laid_window(tree.outsets, super::target::window(&l)),
    );
    // The button's art at W - 28, its 24 px target from W - 31.
    let w = i32::from(served.0);
    assert_eq!(
        field[0] + field[2],
        w - 31,
        "name field box {field:?} in {w}"
    );
}
