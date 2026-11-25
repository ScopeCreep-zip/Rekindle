//! Plan SK17-6: every built-in's served Community window wears the frame
//! its group-chat window wears (the chat window's), node for node and art
//! for art, on the served shell and on the served frame.

use rekindle_types::dto::skin::SkinWindow;

use super::compose;
use crate::{builtin, manifest, package, serve, validate};

/// The built-ins and the window each one's Community window is composed
/// from: Total Chaos lays no chat window (its package holds only `main`),
/// so its Community window falls to its buddy list (plan SK17's order).
const SOURCES: [(&str, SkinWindow); 11] = [
    ("symbiosis", SkinWindow::Chat),
    ("xfire-classic", SkinWindow::Chat),
    ("combat", SkinWindow::Chat),
    ("alloy", SkinWindow::Chat),
    ("onyx", SkinWindow::Chat),
    ("simple-x", SkinWindow::Chat),
    ("spacecowboy", SkinWindow::Chat),
    ("stealthx", SkinWindow::Chat),
    ("total-chaos", SkinWindow::Main),
    ("xglow", SkinWindow::Chat),
    ("xslimfire", SkinWindow::Chat),
];

#[test]
fn every_builtin_serves_its_community_window_in_its_chat_windows_frame() {
    assert_eq!(SOURCES.len(), builtin::PACKS.len());
    for (id, want) in SOURCES {
        let pack = builtin::PACKS.iter().find(|p| p.id == id).unwrap();
        let c = validate::run(pack.bytes).unwrap();
        let address = c.address();
        let p = package::read(&c.bytes).unwrap();
        let m = manifest::served(&p).unwrap();
        let mut uncomposed = m.clone();
        uncomposed.windows.remove(&SkinWindow::Community);
        assert_eq!(compose::source(&uncomposed), Some(want), "{id}");
        assert_eq!(
            m.windows.get(&SkinWindow::Community),
            m.windows.get(&want),
            "{id}: the Community layout is its source's"
        );

        // The served shells: the Community window's (and each of its
        // popups') are the source window's, nodes, roles, art and minimum.
        let sheet = serve::stylesheet(&c.bytes, &address).unwrap();
        let of = |w: SkinWindow| -> Vec<_> {
            sheet
                .shells
                .iter()
                .filter(|s| s.window == w)
                .cloned()
                .map(|mut s| {
                    s.window = want;
                    s
                })
                .collect()
        };
        let source = of(want);
        assert!(
            source.iter().any(|s| s.popup.is_none()),
            "{id}: the source lays a tree"
        );
        assert_eq!(of(SkinWindow::Community), source, "{id}: shells");
        assert!(
            sheet.css.contains("[data-skin-window=\"community\"]"),
            "{id}: the Community window's rules"
        );

        // The served frames: the same layout and tile masks.
        let frames = serve::frames(&c.bytes, &address).unwrap();
        let frame = |w: SkinWindow| {
            frames
                .iter()
                .find(|f| f.window == w)
                .map(|f| (f.layout.clone(), f.masks.clone()))
        };
        assert!(frame(want).is_some(), "{id}");
        assert_eq!(frame(SkinWindow::Community), frame(want), "{id}: frame");
    }
}

/// A pack that lays its own Community window keeps it, and one that lays
/// no window the Community window could wear gets none.
#[test]
fn an_authored_community_window_is_kept_and_nothing_is_invented() {
    let pack = builtin::PACKS.iter().find(|p| p.id == "combat").unwrap();
    let p = package::read(pack.bytes).unwrap();
    let mut m = manifest::served(&p).unwrap();
    let main = m.windows[&SkinWindow::Main].clone();
    m.windows.insert(SkinWindow::Community, main.clone());
    compose::community(&mut m);
    assert_eq!(m.windows[&SkinWindow::Community], main);
    m.windows.clear();
    compose::community(&mut m);
    assert!(m.windows.is_empty());
}
