//! Plan SK15b: a content text keeps its fit and the chains on it stay
//! symbolic in the stored pack (XML and INI importers), and the Host serves
//! every tree numeric, each content text as wide as its served words.

use rekindle_types::dto::skin::{SkinWindow, WindowLayout};
use rekindle_types::dto::skin_tree::{Anchor, NodeSpan, SkinNode, SkinTree};

use super::content::{chains, width, word_width, Link};
use super::golden_tests::main_layout;
use super::test_support::find;
use crate::{builtin, manifest, package};

/// The built-in's main window as stored, before the Host's passes.
fn stored_main(id: &str) -> WindowLayout {
    let (_, b) = builtin::find(&format!("builtin:{id}")).unwrap();
    let p = package::read(b.bytes).unwrap();
    manifest::parse(&p)
        .unwrap()
        .windows
        .remove(&SkinWindow::Main)
        .unwrap()
}

/// The first node under `nodes` served with a chain, except a chain whose
/// base resolves through a [`NodeSpan::Share`] (plan SK17-3, lead ruling):
/// a share's far edge depends on the window, so no px start exists to
/// serve and the stylesheet keeps it as `calc(...)` (Combat's chat
/// `Divider`, `Y=ChatArea.BOTTOM` under `ChatArea`'s `ResizeY=80`,
/// `CombatChatSkin.ini:291-334`).
fn any_after(nodes: &[SkinNode]) -> Option<&str> {
    let axes: [fn(&SkinNode) -> &NodeSpan; 2] = [|n| &n.x, |n| &n.y];
    nodes.iter().find_map(|n| {
        axes.iter()
            .any(|ax| {
                ax(n)
                    .after()
                    .is_some_and(|(sib, _)| !through_share(nodes, sib, *ax))
            })
            .then_some(n.id.as_str())
            .or_else(|| any_after(&n.children))
    })
}

/// Whether the chain on sibling `name` of `nodes` bottoms out at a share
/// on the axis `ax` reads: `name` is one, or is chained on one.
fn through_share(nodes: &[SkinNode], name: &str, ax: fn(&SkinNode) -> &NodeSpan) -> bool {
    let mut name = name;
    for _ in 0..nodes.len() {
        let Some(sib) = nodes.iter().find(|s| s.id.eq_ignore_ascii_case(name)) else {
            return false;
        };
        match ax(sib) {
            NodeSpan::Share { .. } => return true,
            span => match span.after() {
                Some((next, _)) => name = next,
                None => return false,
            },
        }
    }
    false
}

/// Onyx (`mainwindow.xml:42`, `X="XfireMenuText.RIGHT+10"`) and Combat
/// (`combat.ini`, `FileMenuText.RIGHT+12`): each title is a content text
/// with its fit, and the next title's chain on it is kept.
#[test]
fn a_content_text_keeps_its_fit_and_the_chain_on_it_stays_symbolic() {
    for (pack, first, next, gap) in [
        ("onyx", "XfireMenuText", "ToolsMenuText", 10),
        ("onyx", "ToolsMenuText", "HelpMenuText", 10),
        ("combat", "FileMenuText", "ToolsMenuText", 12),
    ] {
        let tree = stored_main(pack).tree.unwrap();
        let a = find(&tree.root, first).unwrap();
        let b = find(&tree.root, next).unwrap();
        assert!(a.fit.is_some() && b.fit.is_some(), "{pack} {first} {next}");
        assert_eq!(b.x.after(), Some((first, gap)), "{pack} {next}");
    }
}

/// No served tree of any built-in window or popup carries a chain, but for
/// one whose base resolves through a share ([`any_after`]).
#[test]
fn no_served_tree_carries_a_chain() {
    for b in builtin::PACKS {
        let p = package::read(b.bytes).unwrap();
        let m = manifest::served(&p).unwrap();
        for (w, layout) in &m.windows {
            let trees = layout
                .tree
                .iter()
                .chain(layout.popups.values().map(|p| &p.tree));
            for t in trees {
                assert_eq!(any_after(&t.root), None, "{} {w:?}", b.id);
            }
        }
    }
}

/// The box of `id` in `tree` laid at the layout's minimum, before the text
/// passes cut it: the content pass's own result.
fn content_box(layout: &WindowLayout, tree: &SkinTree, id: &str) -> [i32; 4] {
    let (w, h) = (i32::from(layout.min_width), i32::from(layout.min_height));
    tree.boxes(w, h)
        .into_iter()
        .find(|(n, _)| n.id == id)
        .map(|(_, b)| b)
        .unwrap()
}

/// Each title of Onyx and Combat is served as wide as its words ("Rekindle"
/// over Xfire's 25 px Combat `File`), and the next title starts its gap
/// past that width.
#[test]
fn titles_hold_their_words_and_the_next_title_follows() {
    for (pack, first, words, next, gap) in [
        ("onyx", "XfireMenuText", "Rekindle", "ToolsMenuText", 10),
        ("onyx", "ToolsMenuText", "Tools", "HelpMenuText", 10),
        ("combat", "FileMenuText", "Rekindle", "ToolsMenuText", 12),
    ] {
        let mut layout = stored_main(pack);
        super::content::serve(&mut layout);
        let tree = layout.tree.clone().unwrap();
        let fit = find(&tree.root, first).and_then(|n| n.fit).unwrap();
        let a = content_box(&layout, &tree, first);
        let b = content_box(&layout, &tree, next);
        assert_eq!(a[2], i32::from(width(fit, words)), "{pack} {first} {a:?}");
        assert!(a[2] > word_width(words), "{pack} {first} {a:?}");
        assert_eq!(b[0], a[0] + a[2] + gap, "{pack} {next} {b:?} after {a:?}");
        let served = main_layout(pack);
        let served = served.tree.as_ref().unwrap();
        assert_eq!(any_after(&served.root), None, "{pack}");
    }
}

/// Plan SK15b-2: XGlow's status row follows its runtime name on y
/// (`Xglow.ini:186-217`): `StatusMiddle` at `UserName.BOTTOM+2` (a far-edge
/// chain on the content text) and `Status` at `StatusMiddle.TOP+1` (a
/// near-edge chain on a tile that follows it) both chain `After` the name,
/// the near edge's offset added to its sibling's gap, whichever parent edge
/// their numeric spans counted from.
#[test]
fn a_near_edge_chain_on_a_following_tile_follows_the_same_sibling() {
    let fixed = |at| NodeSpan::Fixed { at, size: 17 };
    let links = [
        Link {
            id: "UserName",
            fit: true,
            chain: [None, None],
        },
        Link {
            id: "StatusMiddle",
            fit: false,
            chain: [None, Some(("UserName", 2, true))],
        },
        Link {
            id: "Status",
            fit: false,
            chain: [None, Some(("StatusMiddle", 1, false))],
        },
    ];
    let mut spans = vec![
        (fixed(Anchor::Start(20)), fixed(Anchor::End(133))),
        (fixed(Anchor::Start(33)), fixed(Anchor::End(114))),
        (fixed(Anchor::Start(39)), fixed(Anchor::End(116))),
    ];
    chains(&links, &mut spans);
    assert_eq!(spans[1].1.after(), Some(("UserName", 2)), "{:?}", spans[1]);
    assert_eq!(spans[2].1.after(), Some(("UserName", 3)), "{:?}", spans[2]);
    assert_eq!(spans[2].0, fixed(Anchor::Start(39)), "x is not chained");
}
