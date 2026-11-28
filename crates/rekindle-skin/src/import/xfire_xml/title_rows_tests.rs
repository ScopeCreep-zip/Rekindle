use super::*;
use crate::import::tree;

fn sources(pairs: &[(&'static str, &str)]) -> BTreeMap<&'static str, String> {
    pairs.iter().map(|(r, n)| (*r, (*n).to_owned())).collect()
}

/// xSLIMFIRE's layout, cut to the tiles the rule reads.
const SLIM: &str = r#"<Tile Name="W" Component="MainWindow">
        <Tile Name="MainTitleBar" X="0" Y="0" Component="MainTitleBar">
            <Tile Name="XfireLogo" X="20" Y="38" Component="XfireLogo"></Tile>
            <Tile Name="CloseBox" JustX="RIGHT" X="19" Y="4" Component="CloseBox" />
        </Tile>
        <Tile Name="BaseBoard" Y="0" JustY="BOTTOM" Component="BaseBoard">
            <Tile Name="AddButton" Y="5" Component="AddButton"></Tile>
        </Tile>
        <Tile Name="StatusWrapper" X="96" Y="39" Component="StatusWrapper"></Tile>
        <Tile Name="DownArrow" X="21" JustX="RIGHT" Y="37" Component="DownArrow"></Tile>
        <Tile Name="Gripper" Y="MainTitleBar.BOTTOM-15" Component="BottomBar"></Tile>
    </Tile>"#;

fn slim() -> Node {
    tree::parse("xSLIMFIREmainwindow.xml", SLIM.as_bytes()).expect("xml")
}

#[test]
fn the_strip_ends_where_another_slots_art_is_laid() {
    let s = sources(&[
        ("titlebar", "MainTitleBar"),
        ("titlebar.btn.close", "CloseBox"),
        ("statusbar", "StatusWrapper"),
        ("statusbar.arrow", "DownArrow"),
        ("buddylist.action-button", "AddButton"),
        ("buddylist.actionbar", "BottomBar"),
    ]);
    assert_eq!(cut(&slim(), &s, 106), Some(37));
}

#[test]
fn unmapped_art_and_bottom_placed_tiles_keep_the_whole_strip() {
    // The logo maps to no slot; AddButton sits in a bottom-placed tile;
    // the gripper's Y is an expression.
    // The title button at Y=4 is the title's own.
    let s = sources(&[
        ("titlebar", "MainTitleBar"),
        ("titlebar.btn.close", "CloseBox"),
        ("buddylist.action-button", "AddButton"),
        ("buddylist.actionbar", "BottomBar"),
    ]);
    assert_eq!(cut(&slim(), &s, 106), None);
}

/// Symbiosis's layout (`MainWindow.xml:35,48–57`), cut to the tiles
/// the rule reads.
const SYMBIOSIS: &str = r#"<Tile Name="MainWindow" Component="MainWindow">
        <Tile Name="TopRow" Z="1" ResizeX="100" Component="MainTopRow" />
        <Tile Name="InfoBar" Y="5" Z="24" Component="InfoBar">
            <Tile Name="XfireLogo" X="4" Y="5" Component="XfireLogo" />
            <Tile Name="ButtonClose" X="5" Y="3" JustX="Right" Component="ButtonClose" />
        </Tile>
    </Tile>"#;

#[test]
fn a_tile_holding_the_title_buttons_is_the_titles() {
    let s = sources(&[
        ("titlebar", "MainTopRow"),
        ("titlebar.btn.close", "ButtonClose"),
        ("buddylist.infobar", "InfoBar"),
    ]);
    let layout = tree::parse("MainWindow.xml", SYMBIOSIS.as_bytes()).expect("xml");
    assert_eq!(cut(&layout, &s, 13), None);
}

#[test]
fn crop_keeps_the_title_rows_of_every_state() {
    let s = sources(&[
        ("titlebar", "MainTitleBar"),
        ("statusbar.arrow", "DownArrow"),
    ]);
    let img = RgbaImage::from_fn(30, 106, |_, y| {
        rekindle_image::Rgba([0, 0, 0, u8::try_from(y).unwrap_or(0)])
    });
    let mut controls = vec![Control {
        region: "titlebar",
        fit: rekindle_types::dto::skin::RegionFit::Stretch,
        images: [
            (rekindle_types::dto::skin::RegionState::Normal, img.clone()),
            (rekindle_types::dto::skin::RegionState::WindowInactive, img),
        ]
        .into(),
    }];
    assert_eq!(crop(&slim(), &mut controls, &s), Some(37));
    for img in controls[0].images.values() {
        assert_eq!(img.dimensions(), (30, 37));
        assert_eq!(img.get_pixel(0, 36).0[3], 36);
    }
}
