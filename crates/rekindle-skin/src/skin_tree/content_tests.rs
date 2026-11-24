//! Plan SK15b: a content text keeps its fit and the chains on it stay
//! symbolic in the stored pack (XML and INI importers), and the Host serves
//! every tree numeric, each content text as wide as its served words.

use rekindle_types::dto::skin::{SkinWindow, WindowLayout};
use rekindle_types::dto::skin_tree::{SkinNode, SkinTree};

use super::content::{width, word_width};
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

fn any_after(nodes: &[SkinNode]) -> Option<&str> {
    nodes.iter().find_map(|n| {
        (n.x.after().is_some() || n.y.after().is_some())
            .then_some(n.id.as_str())
            .or_else(|| any_after(&n.children))
    })
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

/// No served tree of any built-in window or popup carries a chain.
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
