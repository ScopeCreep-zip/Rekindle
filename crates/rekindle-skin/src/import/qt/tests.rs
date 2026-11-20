//! Synthetic fixtures only: every theme, stylesheet, PNG and SVG is built
//! here (no third-party theme is committed; plan E7.7.3 *Decided
//! (licence)*).

use resvg::usvg::{Group, Node};

use super::*;
use crate::import::pidgin::tests::{has, png};
use crate::import::Imported;
use crate::validate;

fn svg(body: &str, w: u32, h: u32) -> Vec<u8> {
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="{w}" height="{h}">{body}</svg>"#
    )
    .into_bytes()
}

const DOT: &str = r##"<circle cx="8" cy="8" r="6" fill="#53d769"/>"##;

const MUMBLE_QSS: &str = r"/* synthetic */
ApplicationPalette {
  qproperty-window: #2e2e2e;
  qproperty-windowtext: #eee;
  qproperty-windowtext_disabled: #484848;
  qproperty-base: #191919;
  qproperty-highlight: #298ce1;
  qproperty-link: #39a5dd;
  qproperty-midlight: transparent;
  qproperty-button: #444;
  font-size: 9pt;
}
QObject { color: palette(text); background-image: url(skin:controls/x.svg); }
";

fn mumble_tree() -> Tree {
    let mut t = Tree::new();
    t.insert(
        "Synth/theme.ini".into(),
        b"[theme]\nname=Synth\nstyles=dark,lite\n[dark]\nname=Dark\nqss=Dark.qss\n[lite]\nqss=Lite.qss\n"
            .to_vec(),
    );
    t.insert("Synth/Dark.qss".into(), MUMBLE_QSS.into());
    t.insert("Synth/status/online.svg".into(), svg(DOT, 16, 16));
    t.insert("Synth/away.png".into(), png(16));
    t.insert("Synth/talking_on.svg".into(), svg(DOT, 16, 16));
    t
}

const TS3_QSS: &str = r"/*
 * Global overrides for QPalette::ColorRole values:
 * QPalette::Window = #1C2538;
 * QPalette::WindowText = #E0E0E0;
 * QPalette::Link = #1C82CC;
 * QPalette::Midlight = #333;
 *
 * CustomColor::ClientFriend    = #00A22F;
 */
QTreeView { qproperty-indentation: 18; }
QMainWindow #InfoFrame { background-image: url('styles:synth/media/logo.png'); }
";

fn ts3_tree() -> Tree {
    let mut t = Tree::new();
    t.insert(
        "package.ini".into(),
        b"Name        = Synthetic\nType        = Style\n".to_vec(),
    );
    t.insert("styles/synth.qss".into(), TS3_QSS.into());
    t.insert(
        "styles/synth_darkmode.qss".into(),
        b"/* QPalette::AlternateBase = #333; */".to_vec(),
    );
    t.insert("styles/synth/media/busy.png".into(), png(16));
    t.insert("styles/synth/media/logo.png".into(), png(16));
    t.insert("styles/synth/media/platforms/Linux.png".into(), png(16));
    t.insert(
        "styles/synth/clientinfo.tpl".into(),
        b"%%CLIENT_NAME%%".to_vec(),
    );
    t
}

fn imported(outcome: Outcome) -> Imported {
    match outcome {
        Outcome::Passed(i) => i,
        Outcome::Draft(d) => {
            assert!(d.unsettled.is_none(), "unsettled: {:?}", d.unsettled);
            d.suggested.expect("a draft with every failure suggested")
        }
    }
}

#[test]
fn a_minimal_mumble_palette_imports_and_validates() {
    let i = imported(mumble::import(&mumble_tree(), "synth-mumble").unwrap());
    let c = validate::run(&i.canonical.bytes).unwrap();
    assert_eq!(c.address(), i.canonical.address());
    let read = mumble::read(&mumble_tree()).unwrap();
    assert_eq!(
        read.icons.keys().map(String::as_str).collect::<Vec<_>>(),
        ["status.away", "status.online"]
    );
    let n = &i.notice;
    assert!(has(n, "Dark.qss window", "token bg.panel"), "{n}");
    assert!(has(n, "Dark.qss windowtext", "token text.primary"), "{n}");
    assert!(has(n, "Dark.qss link", "token link"), "{n}");
    assert!(has(n, "windowtext_disabled", "no disabled-state"), "{n}");
    assert!(has(n, "midlight = transparent", "bevel"), "{n}");
    assert!(has(n, "button = #444", "no button"), "{n}");
    assert!(has(n, "Dark.qss font-size", "not a qproperty"), "{n}");
    assert!(has(n, "style lite", "one palette per pack"), "{n}");
    assert!(has(n, "talking_on.svg", "not a status slot"), "{n}");
    assert!(has(n, "widget rules", "never applied"), "{n}");
    let again = imported(mumble::import(&mumble_tree(), "synth-mumble").unwrap());
    assert_eq!(again.canonical.bytes, i.canonical.bytes);
    assert_eq!(again.notice, i.notice);
}

#[test]
fn a_minimal_teamspeak_qss_imports_and_validates() {
    let i = imported(teamspeak::import(&ts3_tree(), "synth-ts3").unwrap());
    let c = validate::run(&i.canonical.bytes).unwrap();
    assert_eq!(c.address(), i.canonical.address());
    let n = &i.notice;
    assert!(has(n, "package.ini Name = Synthetic", ""), "{n}");
    assert!(has(n, "synth.qss Window", "token bg.panel"), "{n}");
    assert!(has(n, "synth.qss WindowText", "token text.primary"), "{n}");
    assert!(has(n, "synth.qss Link", "token link"), "{n}");
    assert!(has(n, "Midlight = #333", "bevel"), "{n}");
    assert!(has(n, "CustomColor::ClientFriend", "no friend"), "{n}");
    assert!(has(n, "synth_darkmode.qss", "variant"), "{n}");
    assert!(has(n, "media/busy.png -> status.busy", ""), "{n}");
    assert!(has(n, "media/logo.png", "not a status slot"), "{n}");
    assert!(has(n, "platforms/Linux.png", "not a direct media/"), "{n}");
    assert!(has(n, "clientinfo.tpl", "never rendered"), "{n}");
    // "QPalette::ColorRole values:" is prose, not an override.
    assert!(!n.contains("ColorRole"), "{n}");
}

#[test]
fn a_palette_that_fails_contrast_is_a_draft_with_named_failures() {
    let mut t = ts3_tree();
    let low = TS3_QSS.replace("#E0E0E0", "#2A3450");
    t.insert("styles/synth.qss".into(), low.into_bytes());
    let Outcome::Draft(d) = teamspeak::import(&t, "synth-ts3").unwrap() else {
        panic!("a 1.2:1 window text passes");
    };
    assert!(d.failures.iter().any(|c| c.token == "text.primary"));
    assert!(validate::run(&d.package).is_err());
    assert!(validate::run(&d.suggested.unwrap().canonical.bytes).is_ok());
}

fn has_image(g: &Group) -> bool {
    g.children().iter().any(|n| match n {
        Node::Image(_) => true,
        Node::Group(g) => has_image(g),
        _ => false,
    })
}

#[test]
fn a_file_href_is_refused_and_never_read() {
    // A real PNG on disk: the default usvg resolver would read it into an
    // image node (`usvg-image.rs:85-99`); ours records it and reads nothing.
    let dir = std::env::temp_dir().join(format!("rekindle-skin-svg-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("local.png");
    std::fs::write(&file, png(8)).unwrap();
    let href = file.to_str().unwrap().to_owned();
    let body = format!(r#"{DOT}<image href="{href}" width="8" height="8"/>"#);
    let (tree, refused) = svg::parse(&svg(&body, 16, 16)).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(!has_image(tree.root()), "the local file was read");
    assert_eq!(refused, [href]);

    let body = format!(r#"{DOT}<image xlink:href="file:///etc/passwd" width="8" height="8"/>"#);
    let raster = svg::rasterise(&svg(&body, 16, 16)).unwrap();
    assert_eq!(raster.refused, ["file:///etc/passwd"]);
    assert!(images::check("icons/status.online.png", &raster.png).is_ok());
}

#[test]
fn external_and_data_hrefs_are_refused_and_the_icon_still_draws() {
    let body = format!(
        r#"{DOT}<image href="https://example.invalid/x.png" width="8" height="8"/><image href="data:image/png;base64,iVBORw0KGgo=" width="8" height="8"/>"#
    );
    let mut t = mumble_tree();
    t.insert("Synth/status/online.svg".into(), svg(&body, 16, 16));
    let read = mumble::read(&t).unwrap();
    assert!(read.icons.contains_key("status.online"));
    let refs: Vec<&str> = read.refused.iter().map(|(_, h)| h.as_str()).collect();
    assert_eq!(refs, ["https://example.invalid/x.png", "data:image/png"]);
}

#[test]
fn an_svg_past_the_pixel_limit_is_refused_before_rasterising() {
    let (tree, _) = svg::parse(&svg(DOT, 2000, 2040)).unwrap();
    let err = svg::raster_size(&tree).unwrap_err();
    assert!(err.contains("past 4000000 pixels"), "{err}");
    let err = svg::rasterise(&svg(DOT, 5000, 16)).unwrap_err();
    assert!(err.contains("outside 1..=2048"), "{err}");
    let mut t = mumble_tree();
    t.insert("Synth/status/online.svg".into(), svg(DOT, 2000, 2040));
    let n = &imported(mumble::import(&t, "synth-mumble").unwrap()).notice;
    assert!(has(n, "online.svg", "SVG refused"), "{n}");
}

#[test]
fn text_dtds_and_stylesheets_are_refused_by_the_prescan() {
    let text = svg("<text>hi</text>", 16, 16);
    assert!(svg::parse(&text).unwrap_err().contains("<text>"));
    let dtd = br#"<?xml version="1.0"?><!DOCTYPE svg [<!ENTITY a "aaaa">]><svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"/>"#;
    assert!(svg::parse(dtd).is_err());
    let css = br#"<?xml-stylesheet href="file:///etc/x.css"?><svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"/>"#;
    assert!(svg::parse(css).unwrap_err().contains("xml-stylesheet"));
}

#[test]
fn a_teamspeak_package_ini_author_is_credited() {
    let mut tree = ts3_tree();
    tree.insert(
        "package.ini".into(),
        b"Name = Synthetic\nType = Style\nAuthor = Ann\n".to_vec(),
    );
    let read = teamspeak::read(&tree).unwrap();
    let m = manifest(&tree, "synth-ts3", &read, "", SkinBase::Dark);
    assert_eq!(m.credits.expect("credits").authors, vec!["Ann".to_owned()]);
    let mumble = mumble::read(&mumble_tree()).unwrap();
    let m = manifest(&mumble_tree(), "synth-mumble", &mumble, "", SkinBase::Dark);
    assert!(m.credits.expect("credits").authors.is_empty());
}

const BACKGROUND_QSS: &str = r"ApplicationPalette { qproperty-window: #101010; qproperty-windowtext: #eee; }
/* QPushButton { background-image: url(skin:commented.png); } */
QMainWindow { background-image: url(skin:bg/window.png); background-repeat: repeat; }
QTreeView { background-image: url('skin:bg/list.png'); }
QPushButton { background-image: url(skin:bg/button.png); }
QPushButton:hover { background-image: url(skin:bg/button.svg); }
QPushButton:pressed { background-image: url(http://example.invalid/b.png); }
QPushButton:disabled { background-image: url(data:image/png;base64,AAAA); }
QStatusBar { background-image: url(skin:../outside.png); }
QScrollBar::handle:vertical { background-image: url(:/builtin/handle.png); }
QScrollBar:vertical { background-image: url(skin:bg/huge.png); }
QToolTip { background-image: url(skin:bg/window.png); }
";

fn background_tree(qss: &str) -> Tree {
    let mut t = Tree::new();
    t.insert(
        "theme.ini".into(),
        b"[theme]\nstyles=dark\n[dark]\nqss=Dark.qss\n".to_vec(),
    );
    t.insert("Dark.qss".into(), qss.as_bytes().to_vec());
    t.insert("bg/window.png".into(), png(16));
    t.insert("bg/list.png".into(), png(8));
    t.insert("bg/button.png".into(), png(24));
    t.insert(
        "bg/button.svg".into(),
        svg(r##"<rect width="24" height="24" fill="#335"/>"##, 24, 24),
    );
    let side = crate::limits::MAX_IMAGE_SIDE + 1;
    let wide = rekindle_image::RgbaImage::new(side, 1);
    t.insert(
        "bg/huge.png".into(),
        rekindle_image::encode_png(&rekindle_image::DynamicImage::ImageRgba8(wide)).unwrap(),
    );
    t
}

#[test]
fn qss_backgrounds_inside_the_theme_fill_their_slots() {
    use rekindle_types::dto::skin::{RegionFit, RegionState};
    let tree = background_tree(BACKGROUND_QSS);
    let i = imported(mumble::import(&tree, "synth-bg").unwrap());
    validate::run(&i.canonical.bytes).unwrap();
    let read = mumble::read(&tree).unwrap();
    let states = |slot: &str| read.regions[slot].keys().copied().collect::<Vec<_>>();
    assert_eq!(
        read.regions.keys().copied().collect::<Vec<_>>(),
        ["button.secondary", "window.frame"]
    );
    assert_eq!(states("window.frame"), [RegionState::Normal]);
    assert_eq!(
        states("button.secondary"),
        [RegionState::Normal, RegionState::Hover]
    );
    // The window's own rule wins over the list's (priority order).
    assert_eq!(
        read.regions["window.frame"][&RegionState::Normal].2,
        "bg/window.png"
    );
    let spec = background::spec("button.secondary", &read.regions["button.secondary"]);
    assert_eq!(spec.fit, RegionFit::Tile);
    assert_eq!(spec.text_backing.as_deref(), Some("bg.panel"));
    let n = &i.notice;
    assert!(
        has(n, "bg/list.png as window.frame normal", "already set"),
        "{n}"
    );
    assert!(
        has(
            n,
            "QPushButton:pressed background-image",
            "not a file inside"
        ),
        "{n}"
    );
    assert!(
        has(
            n,
            "QPushButton:disabled background-image",
            "not a file inside"
        ),
        "{n}"
    );
    assert!(
        has(n, "QStatusBar background-image", "leaves the theme"),
        "{n}"
    );
    assert!(
        has(n, "handle:vertical background-image", "not a file inside"),
        "{n}"
    );
    assert!(
        has(n, "bg/huge.png as scrollbar.track normal", "validator"),
        "{n}"
    );
    assert!(
        has(n, "QToolTip background-image", "not a whole widget"),
        "{n}"
    );
    assert!(!n.contains("commented.png"), "{n}");
}

#[test]
fn a_qss_url_outside_the_bounds_is_refused() {
    let long = format!("skin:{}.png", "a".repeat(background::MAX_URL));
    for (front, root, url) in [
        (Front::Mumble, "", long.as_str()),
        (Front::Mumble, "", "https://example.invalid/x.png"),
        (Front::Mumble, "", "data:image/png;base64,AAAA"),
        (Front::Mumble, "", "skin:/etc/x.png"),
        (Front::Mumble, "", "skin:a/../../x.png"),
        (Front::Mumble, "", "bg/window.png"),
        (Front::TeamSpeak3, "", "skin:x.png"),
    ] {
        assert!(background::resolve(front, root, url).is_err(), "{url}");
    }
    assert_eq!(
        background::resolve(Front::TeamSpeak3, "pack/", "styles:s/media/a.png").as_deref(),
        Ok("pack/styles/s/media/a.png")
    );
    let many: String = (0..=background::MAX_BACKGROUNDS)
        .map(|_| "QPushButton { background-image: url(skin:bg/button.png); }\n")
        .collect();
    let read = mumble::read(&background_tree(&format!(
        "ApplicationPalette {{ qproperty-window: #101010; }}\n{many}"
    )))
    .unwrap();
    assert!(read
        .dropped
        .iter()
        .any(|d| d.why.contains("past 64 backgrounds")));
}
