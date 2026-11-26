//! The dialog roles an Xfire import writes (plan E7.7.3 item 1, E7.7.7).

use std::collections::BTreeMap;

use rekindle_image::{Rgba, RgbaImage};
use rekindle_types::dto::skin::{Insets, RegionFit, RegionState, TokenValue};

use super::dialog_roles::{art_mean, strip_ends, with_art};
use super::walk::color;
use super::{xfire_xml, Control, Tree};
use crate::registry::{dialog, Base};

fn rgb(tokens: &BTreeMap<String, TokenValue>, id: &str) -> [u8; 3] {
    match &tokens[id] {
        TokenValue::Color { r, g, b, .. } => [*r, *g, *b],
        v => panic!("{id}: {v:?}"),
    }
}

/// The colours every Xfire pack sets (its list, chat view, info view,
/// friend text and selected row).
fn pack_tokens() -> BTreeMap<String, TokenValue> {
    [
        ("bg.primary", [10, 10, 10]),
        ("bg.panel", [20, 20, 20]),
        ("bg.secondary", [40, 40, 40]),
        ("text.base", [231, 231, 231]),
        ("accent.fill", [23, 124, 193]),
    ]
    .into_iter()
    .map(|(k, c)| (k.to_owned(), color(c)))
    .collect()
}

/// A strip whose left 3 columns are red, right 2 blue, the rest green, with
/// one transparent pixel at each end that must not count.
fn strip() -> RgbaImage {
    let mut img = RgbaImage::from_pixel(10, 2, Rgba([0, 255, 0, 255]));
    for y in 0..2 {
        for x in 0..3 {
            img.put_pixel(x, y, Rgba([255, 0, 0, 255]));
        }
        for x in 8..10 {
            img.put_pixel(x, y, Rgba([0, 0, 255, 255]));
        }
    }
    img.put_pixel(0, 0, Rgba([255, 255, 255, 0]));
    img.put_pixel(9, 1, Rgba([255, 255, 255, 0]));
    img
}

const INSETS: RegionFit = RegionFit::NineSlice(Insets {
    top: 0,
    right: 2,
    bottom: 0,
    left: 3,
});

#[test]
fn the_strip_ends_are_the_mean_of_its_end_pieces() {
    assert_eq!(
        strip_ends(&strip(), INSETS),
        Some(([255, 0, 0], [0, 0, 255]))
    );
    // No insets: the outer eighths (one column of ten).
    assert_eq!(
        strip_ends(&strip(), RegionFit::Stretch),
        Some(([255, 0, 0], [0, 0, 255]))
    );
}

#[test]
fn a_pack_with_a_strip_takes_its_title_ground_from_it() {
    let control = Control {
        region: "titlebar",
        fit: INSETS,
        images: BTreeMap::from([(RegionState::Normal, strip())]),
    };
    let t = with_art(pack_tokens(), &[control]);
    assert_eq!(rgb(&t, "titlebar.start"), [255, 0, 0]);
    assert_eq!(rgb(&t, "titlebar.end"), [0, 0, 255]);
}

/// The menu bar's ground is the mean of the whole bar: 3 red, 5 green and
/// 2 blue columns, less the two transparent pixels (ST1 ruling 3).
#[test]
fn a_pack_with_menu_bar_art_takes_its_menu_ground_from_it() {
    let control = Control {
        region: "buddylist.menubar",
        fit: RegionFit::Stretch,
        images: BTreeMap::from([(RegionState::Normal, strip())]),
    };
    let t = with_art(pack_tokens(), &[control]);
    assert_eq!(art_mean(&strip()), Some([71, 142, 43]));
    assert_eq!(rgb(&t, "menubar.bg"), [71, 142, 43]);
}

/// The import derives a role the pack neither names nor draws by the
/// registry's own rows, so it writes what load serves (ST1 ruling 4).
#[test]
fn a_strip_less_pack_derives_every_role_from_its_own_colours() {
    let mut t = with_art(pack_tokens(), &[]);
    dialog::fill(&mut t, Base::Dark);
    for id in dialog::roles() {
        assert!(t.contains_key(id), "{id} unset");
    }
    // The selected row's colour toward the list ground: (23+10)/2, …
    assert_eq!(rgb(&t, "titlebar.start"), [17, 67, 102]);
    assert_eq!(rgb(&t, "input.bg"), [20, 20, 20]);
    assert_eq!(rgb(&t, "input.text"), [231, 231, 231]);
    assert_eq!(rgb(&t, "scrollbar.track"), [10, 10, 10]);
    // The window ground, which follows the list ground.
    assert_eq!(rgb(&t, "menubar.bg"), [10, 10, 10]);
}

/// An XML pack that names its dialog colours.
fn tree() -> Tree {
    let colours = [
        ("ListBGCG", "10,10,10"),
        ("ChatViewBGCG", "20,20,20"),
        ("InfoViewBGCG", "40,40,40"),
        ("ListFriendTextCG", "231,231,231"),
        ("FriendNameCG", "255,255,255"),
        ("ListColorSelectedCG", "23,124,193"),
        ("ListColorSelectedTextCG", "250,250,250"),
        ("TextFieldBGCG", "30,30,30"),
        ("TextFieldTextCG", "245,245,245"),
        ("MenuTextCG", "143,185,215"),
        ("ToolTipBGCG", "56,56,56"),
        ("ToolTipTextCG", "200,200,200"),
    ]
    .map(|(n, v)| format!("<Color Name=\"{n}\">RGBA({v},0)</Color>"))
    .concat();
    let mut t = Tree::new();
    t.insert(
        "Skin.xml".into(),
        br#"<XfireSkin><Include File="Themes.xml"/></XfireSkin>"#.to_vec(),
    );
    t.insert(
        "Themes.xml".into(),
        format!("<Themes><Theme Name=\"default\"><Colors>{colours}</Colors></Theme></Themes>")
            .into_bytes(),
    );
    t
}

/// Each named colour reaches its role as mapped, before any walk; any walk
/// change to a role is recorded in the NOTICE (ST1 ruling 5).
/// Fail-without: `TextFieldTextCG` mapped back to `NoRole` leaves
/// `input.text` unmapped.
#[test]
fn every_named_dialog_colour_reaches_its_role() {
    let read = xfire_xml::read(&tree()).unwrap();
    for (id, want) in [
        ("input.text", [245, 245, 245]),
        ("input.bg", [30, 30, 30]),
        ("selection.text", [250, 250, 250]),
        ("menubar.text", [143, 185, 215]),
        ("tooltip.bg", [56, 56, 56]),
        ("tooltip.text", [200, 200, 200]),
    ] {
        assert!(read.tokens.contains_key(id), "{id} not mapped");
        assert_eq!(rgb(&read.tokens, id), want, "{id}");
    }
    let imported = xfire_xml::import(&tree(), "fixture", None).unwrap();
    let pkg = crate::package::read(&imported.canonical.bytes).unwrap();
    let m = crate::manifest::parse(&pkg).unwrap();
    let v = &m.variants[0];
    let t = crate::tokens::parse(&v.tokens, pkg.get(&v.tokens).unwrap()).unwrap();
    for id in dialog::roles() {
        assert!(t.contains_key(id), "{id} unset");
    }
    let noted = |id: &str| {
        imported
            .notice
            .lines()
            .any(|l| l.starts_with(&format!("  {id}: #")))
    };
    for c in &imported.changes {
        assert!(noted(&c.token), "{} walked but not in the NOTICE", c.token);
    }
    for (id, mapped) in read.tokens.iter().filter(|(id, _)| dialog::is_role(id)) {
        let walked = imported.changes.iter().any(|c| &c.token == id);
        assert!(walked || &t[id] == mapped, "{id} moved with no walk change");
    }
}
