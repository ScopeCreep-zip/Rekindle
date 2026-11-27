use super::super::super::tree::parse;
use super::*;

/// Symbiosis's "Window Frame" block (`MainWindow.xml`): the top row
/// typed `Left`/`Middle`/`Right`, the bottom row by position.
#[test]
fn a_bottom_row_typed_by_position_is_the_sides_shape() {
    let layout = parse(
        "MainWindow.xml",
        br#"<Tile Name="MainWindow" Component="MainWindow">
<Tile Name="TopRow" Component="MainTopRow"/><Tile Name="LeftSide" Component="MainLeftSide"/>
<Tile Name="RightSide" Component="MainRightSide"/><Tile Name="BottomRow" Component="MainBottomRow"/></Tile>"#,
    )
    .unwrap();
    let components = parse(
        "Components/MainWindow.xml",
        br#"<Components>
<Component Name="MainTopRow"><Images State="Normal"><Image Type="Left">tl</Image><Image Type="Middle">t</Image><Image Type="Right">tr</Image></Images></Component>
<Component Name="MainLeftSide"><Images State="Normal"><Image Type="Middle">l</Image></Images></Component>
<Component Name="MainRightSide"><Images State="Normal"><Image Type="Right">r</Image></Images></Component>
<Component Name="MainBottomRow"><Images State="Normal"><Image Type="BottomLeft">bl</Image><Image Type="Bottom">b</Image><Image Type="BottomRight">br</Image></Images></Component>
</Components>"#,
    )
    .unwrap();
    let names = std::cell::RefCell::new(Vec::new());
    let got = pieces(&layout, &[components], |name, _| {
        names.borrow_mut().push(name.to_owned());
        Ok(RgbaImage::new(2, 2))
    })
    .unwrap()
    .map(|(_, shape, _)| shape);
    assert_eq!(got, Some(Shape::Sides));
    assert_eq!(
        names.into_inner(),
        ["tl", "t", "tr", "l", "r", "bl", "b", "br"]
    );
}

/// XGlow's chat form (`ChatSkin.ini`): typed top and bottom rows, and
/// the middle band as three resizing tiles of one image each.
#[test]
fn rows_with_a_tile_band_are_the_band_shape() {
    let layout = parse(
        "ChatSkin.ini",
        br#"<Tile Name="MainWindow" Component="MainWindow">
<Tile Name="Top" Component="Top"/><Tile Name="Left" ResizeY="100" Component="Left"/>
<Tile Name="Middle" ResizeY="100" Component="Middle"/><Tile Name="Right" ResizeY="100" Component="Right"/>
<Tile Name="Bottom" Component="Bottom"/></Tile>"#,
    )
    .unwrap();
    let components = parse(
        "ChatSkin.ini",
        br#"<Components>
<Component Name="Top"><Images State="Normal"><Image Type="topleft">tl</Image><Image Type="top">t</Image><Image Type="topright">tr</Image></Images></Component>
<Component Name="Left"><Images State="Normal"><Image Type="middle">l</Image></Images></Component>
<Component Name="Middle"><Images State="Normal"><Image Type="middle">m</Image></Images></Component>
<Component Name="Right"><Images State="Normal"><Image Type="middle">r</Image></Images></Component>
<Component Name="Bottom"><Images State="Normal"><Image Type="bottomleft">bl</Image><Image Type="bottom">b</Image><Image Type="bottomright">br</Image></Images></Component>
</Components>"#,
    )
    .unwrap();
    let names = std::cell::RefCell::new(Vec::new());
    let got = pieces(&layout, &[components], |name, _| {
        names.borrow_mut().push(name.to_owned());
        Ok(RgbaImage::new(2, 2))
    })
    .unwrap()
    .map(|(_, shape, _)| shape);
    assert_eq!(got, Some(Shape::Band));
    assert_eq!(
        names.into_inner(),
        ["tl", "t", "tr", "l", "m", "r", "bl", "b", "br"]
    );
}
