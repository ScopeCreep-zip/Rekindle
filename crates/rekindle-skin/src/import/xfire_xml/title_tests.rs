//! The title importer's resolution rules, each on the pack that needs it
//! (plan SK1-S1): Classic v3's summed indent, its name field stretching
//! from a sibling's edge and its close box after that field; Space
//! Cowboy's right-justified stretching bar; the name field's pack colour
//! and 3-slice art; Alloy's INI sections read as tiles.

use std::collections::BTreeMap;

use rekindle_image::{Rgba, RgbaImage};
use rekindle_types::dto::skin::{SkinWindow, TileAxis, WindowLayout};
use rekindle_types::dto::skin_title::{TitleLayout, TitleRole};

use super::super::super::tree::{parse, Node};
use super::super::super::xfire_ini::{ini, title as ini_title};
use super::super::colours::Target;
use super::super::ColourRow;
use super::add;
use crate::shape;

/// A solid image sized by its name: `w×h` (`20x16.gif` → 20 × 16).
fn sized(name: &str, _: Option<&str>) -> RgbaImage {
    let stem = name.split('.').next().unwrap_or_default();
    let (w, h) = stem.split_once('x').unwrap();
    RgbaImage::from_pixel(w.parse().unwrap(), h.parse().unwrap(), Rgba([9, 9, 9, 255]))
}

fn lay(layout: &Node, components: &Node, colours: &[ColourRow]) -> Option<TitleLayout> {
    let solid = RgbaImage::from_pixel(10, 10, Rgba([1, 2, 3, 255]));
    let mut built: (WindowLayout, BTreeMap<String, Vec<u8>>) =
        shape::from_nine(SkinWindow::Main, &std::array::from_fn(|_| solid.clone())).unwrap();
    add(
        SkinWindow::Main,
        layout,
        std::slice::from_ref(components),
        colours,
        sized,
        &mut built,
    )
    .unwrap();
    built.0.title
}

fn xml(text: &str) -> Node {
    parse("t.xml", text.as_bytes()).unwrap()
}

fn part(t: &TitleLayout, role: TitleRole) -> &rekindle_types::dto::skin_title::TitlePart {
    t.parts.iter().find(|p| p.role == role).unwrap()
}

const CLASSIC_COMPONENTS: &str = r#"<Components>
<Component Name="MainTitleBar"><Images><Image Type="Middle">10x51.gif</Image></Images></Component>
<Component Name="XfireLogo"><Images><Image Type="Middle">40x20.gif</Image></Images></Component>
<Component Name="UserNameField"><Images>
<Image Type="Left">5x18.gif</Image><Image Type="Middle">2x18.gif</Image><Image Type="Right">6x18.gif</Image>
</Images></Component>
<Component Name="UserNameText"><Text Color="UserNameCG" Font="Arial" Bold="true" Size="12">UserNameText</Text></Component>
<Component Name="CloseBox" Class="PushButton"><Images><Image Type="Middle">16x18.gif</Image></Images></Component>
</Components>"#;

/// Classic v3 `xfiremainwindow.xml:2-12`.
const CLASSIC: &str = r#"<Tile Name="XfireMainWindow" Component="MainWindow">
<Tile Name="MainTitleBar" X="0" Y="0" IndentLeft="14" IndentRight="13+14" ResizeX="100" Component="MainTitleBar">
<Tile Name="XfireLogo" X="20" Y="20" Component="XfireLogo"/>
<Tile Name="UserNameField" X="XfireLogo.RIGHT+6" Y="XfireLogo.TOP+2" ResizeX="100" IndentRight="48" Component="UserNameField">
<Tile Name="UserNameText" X="13" Y="0" ResizeX="100" IndentRight="13" Component="UserNameText"/>
</Tile>
<Tile Name="CloseBox" X="UserNameField.RIGHT+1" Y="UserNameField.TOP" Component="CloseBox"/>
</Tile></Tile>"#;

fn user_name_cg() -> Vec<ColourRow> {
    vec![ColourRow {
        name: "UserNameCG".to_owned(),
        value: [255, 204, 0],
        target: Target::Shift,
    }]
}

#[test]
fn classic_sums_its_indent_and_lays_the_close_box_after_the_stretching_name_field() {
    let t = lay(&xml(CLASSIC), &xml(CLASSIC_COMPONENTS), &user_name_cg()).unwrap();
    // The bar spans 14 .. W-27; the logo at 14+20; the field from the
    // logo's right edge + 6 to 27 + 48 from the right.
    let name = part(&t, TitleRole::Name);
    assert_eq!(
        name.x,
        TileAxis::Repeat {
            start: 14 + 20 + 40 + 6,
            end: 27 + 48
        }
    );
    assert_eq!((name.y, name.height), (22, 18));
    let close = part(&t, TitleRole::Close);
    assert_eq!(
        close.x,
        TileAxis::End {
            offset: 27 + 48 - 1 - 16
        }
    );
    assert_eq!((close.y, close.width, close.height), (22, 16, 18));
    assert_eq!(part(&t, TitleRole::Logo).x, TileAxis::Start { offset: 34 });
    assert_eq!(t.bottom, 51);
}

#[test]
fn the_name_field_takes_its_pack_colour_inset_and_slices() {
    let t = lay(&xml(CLASSIC), &xml(CLASSIC_COMPONENTS), &user_name_cg()).unwrap();
    let name = part(&t, TitleRole::Name);
    let text = name.text.as_ref().unwrap();
    assert_eq!(text.colour.as_deref(), Some("#ffcc00"));
    assert_eq!(text.inset, [13, 13]);
    assert!(text.bold);
    let s = name.slices.as_ref().unwrap();
    assert!(s.left.contains("title-name--left") && s.right.contains("title-name--right"));
}

/// Space Cowboy `mainwindow.xml`: `MainTitleBar JustX="RIGHT" X="85"
/// ResizeX="100"`, its buttons right-justified in it.
#[test]
fn a_right_justified_stretching_bar_offsets_its_right_edge_by_x() {
    let layout = xml(r#"<Tile Name="W" Component="W">
<Tile Name="MainTitleBar" JustX="RIGHT" X="85" Y="47" ResizeX="100" Component="Bar">
<Tile Name="Close" JustX="RIGHT" X="0" Y="0" Component="Close"/>
</Tile></Tile>"#);
    let components = xml(r#"<Components>
<Component Name="Bar"><Images><Image Type="Middle">10x19.gif</Image></Images></Component>
<Component Name="Close"><Images><Image Type="Middle">15x15.gif</Image></Images></Component>
</Components>"#);
    let t = lay(&layout, &components, &[]).unwrap();
    assert_eq!(part(&t, TitleRole::Close).x, TileAxis::End { offset: 85 });
    assert_eq!(part(&t, TitleRole::Close).y, 47);
}

/// Alloy `Alloy.ini`: `[MainWindow]` lays `Top`, `Logo`, `UserNameField`
/// and `TopButtonBar`, whose `Window<n>` keys lay the three buttons.
#[test]
fn an_ini_skin_file_is_read_as_tiles_and_lays_its_strip() {
    let skin = ini::parse(
        "Alloy.ini",
        b"[MainWindow]\nWindow1.Name=Top\nWindow1.X=0\nWindow1.Y=0\nWindow1.ResizeX=100\n\
Window2.Name=Logo\nWindow2.X=11\nWindow2.Y=11\n\
Window4.Name=UserNameField\nWindow4.X=20\nWindow4.Y=60\n\
Window8.Name=TopButtonBar\nWindow8.JustX=RIGHT\nWindow8.X=7\nWindow8.Y=4\n\
[Top]\nClass=Tile\nGraphics.top=10x40.gif\n\
[Logo]\nClass=PushButton\nGraphics.ColorGroup=LogoCG\nGraphics.middle=30x20.gif\n\
[UserNameField]\nClass=Tile\nGraphics.middle=4x16.gif\nWindow1.Name=UserNameText\nWindow1.X=5\nWindow1.Y=1\n\
[UserNameText]\nClass=Text\nText.color=UserNameCG\nText.font=Arial\nText.bold=true\nText.size=11\n\
[TopButtonBar]\nClass=Tile\nGraphics.middle=56x18.gif\n\
Window1.Name=MinimizeBox\nWindow1.X=1\nWindow1.Y=1\nWindow3.Name=CloseBox\nWindow3.X=37\nWindow3.Y=1\n\
[CloseBox]\nClass=PushButton\nGraphics.middle=17x16.gif\nHover.middle=17x16.gif\nBtnDown.middle=17x16.gif\n\
[MinimizeBox]\nClass=PushButton\nGraphics.middle=17x16.gif\n",
    )
    .unwrap();
    let (layout, components) = ini_title::tree(&skin).unwrap();
    let t = lay(&layout, &components, &user_name_cg()).unwrap();
    // The bar's right edge is 7 from the window's, 56 wide: its left edge
    // is 63 from the right, the close box's 63 - 37 = 26, so its right
    // edge 26 - 17 = 9.
    let close = part(&t, TitleRole::Close);
    assert_eq!(close.x, TileAxis::End { offset: 9 });
    assert_eq!((close.y, close.states.len()), (5, 2));
    assert_eq!(part(&t, TitleRole::Logo).x, TileAxis::Start { offset: 11 });
    // The name field (y 60) is below the 40 px strip: not laid.
    assert!(t.parts.iter().all(|p| p.role != TitleRole::Name));
}
