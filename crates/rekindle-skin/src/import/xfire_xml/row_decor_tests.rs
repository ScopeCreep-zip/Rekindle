use super::super::super::tree::parse;
use super::super::decor::apply;
use super::*;

/// Onyx's `MenuBar` across a window whose side pieces are transparent
/// (kept: it fills the side columns), and a row over the opaque top
/// row (dropped: it adds nothing to the outline).
#[test]
fn a_row_is_kept_only_where_it_adds_to_the_outline() {
    let layout = parse(
        "mainwindow.xml",
        br#"<Tile Name="Main" Component="MainWindow">
<Tile Name="MenuBar" X="0" Y="12" Z="1" ResizeX="100" IndentRight="0" Component="Bar"/>
<Tile Name="Over" X="0" Y="0" Z="1" ResizeX="100" Component="Bar"/></Tile>"#,
    )
    .unwrap();
    let components = parse(
        "components.xml",
        br#"<Components><Component Name="Bar"><Images>
<Image Type="left">l</Image><Image Type="middle">m</Image><Image Type="right">r</Image>
</Images></Component></Components>"#,
    )
    .unwrap();
    let px = |a| rekindle_image::Rgba([1, 2, 3, a]);
    let pieces = std::array::from_fn(|i| {
        RgbaImage::from_pixel(10, 10, px(if i == 3 || i == 5 { 0 } else { 255 }))
    });
    let mut built = shape::from_nine(SkinWindow::Main, &pieces).unwrap();
    apply(
        SkinWindow::Main,
        &layout,
        &[components],
        &[],
        |_, _| Ok(RgbaImage::from_pixel(10, 2, px(255))),
        &mut built,
    )
    .unwrap();
    let got: Vec<_> = built
        .0
        .tiles
        .iter()
        .filter(|t| t.decoration.is_some())
        .map(|t| (t.image.as_str(), t.x, t.y))
        .collect();
    let at = |part: &str| format!("assets/frame/main/row-menubar-{part}.png");
    let y = TileAxis::Start { offset: 12 };
    let (l, m, r) = (at("left"), at("middle"), at("right"));
    let want = vec![
        (l.as_str(), TileAxis::Start { offset: 0 }, y),
        (m.as_str(), TileAxis::Repeat { start: 10, end: 10 }, y),
        (r.as_str(), TileAxis::End { offset: 0 }, y),
    ];
    assert_eq!(got, want);
}

/// xSLIMFIRE's `AdArea` row names `ADBARLEFT`/`ADBARRIGHT`, which its
/// theme never defines: the row lays its middle only and the import
/// goes on; a defined bitmap whose file is missing still stops it.
#[test]
fn an_undefined_side_bitmap_lays_nothing_and_a_missing_file_stops() {
    let layout = parse(
        "mainwindow.xml",
        br#"<Tile Name="Main" Component="MainWindow">
<Tile Name="AdArea" X="0" Y="12" Z="1" ResizeX="100" IndentRight="0" Component="Bar"/></Tile>"#,
    )
    .unwrap();
    let components = parse(
        "components.xml",
        br#"<Components><Component Name="Bar"><Images>
<Image Type="left">ADBARLEFT</Image><Image Type="middle">BLACK</Image><Image Type="right">ADBARRIGHT</Image>
</Images></Component></Components>"#,
    )
    .unwrap();
    let px = |a| rekindle_image::Rgba([1, 2, 3, a]);
    let pieces = std::array::from_fn(|i| {
        RgbaImage::from_pixel(10, 10, px(if i == 3 || i == 5 { 0 } else { 255 }))
    });
    let mut built = shape::from_nine(SkinWindow::Main, &pieces).unwrap();
    let undefined = |name: &str, _: Option<&str>| match name {
        "BLACK" => Ok(RgbaImage::from_pixel(10, 2, px(255))),
        _ => Err(ImportError::UndefinedBitmap {
            name: name.to_owned(),
        }),
    };
    apply(
        SkinWindow::Main,
        &layout,
        std::slice::from_ref(&components),
        &[],
        undefined,
        &mut built,
    )
    .unwrap();
    let got: Vec<_> = built
        .0
        .tiles
        .iter()
        .filter(|t| t.decoration.is_some())
        .map(|t| t.image.as_str())
        .collect();
    assert_eq!(got, ["assets/frame/main/row-adarea-middle.png"]);
    let missing = |name: &str, _: Option<&str>| match name {
        "BLACK" => Ok(RgbaImage::from_pixel(10, 2, px(255))),
        _ => Err(ImportError::MissingFile {
            file: format!("main/{name}.gif"),
        }),
    };
    let mut built = shape::from_nine(SkinWindow::Main, &pieces).unwrap();
    assert!(matches!(
        apply(
            SkinWindow::Main,
            &layout,
            &[components],
            &[],
            missing,
            &mut built
        ),
        Err(ImportError::MissingFile { .. })
    ));
}

/// Space Cowboy's `BRGripper`: stretched (`ResizeX="100"`) but capped by
/// its component's `MaxWidth="38"`, so it is no row (laid across a wide
/// window its dots made 4499 silhouette rectangles).
#[test]
fn a_stretch_its_component_caps_is_no_row() {
    let layout = parse(
        "mainwindow.xml",
        br#"<Tile Name="Main" Component="MainWindow">
<Tile Name="BRGripper" JustX="RIGHT" X="19" JustY="BOTTOM" Y="2" ResizeX="100" Z="40" Component="BRGripper"/></Tile>"#,
    )
    .unwrap();
    let components = parse(
        "components.xml",
        br#"<Components><Component Name="BRGripper" Class="Tile" MinWidth="38" MaxWidth="38"><Images>
<Image Type="middle">grip</Image></Images></Component></Components>"#,
    )
    .unwrap();
    let px = |a| rekindle_image::Rgba([1, 2, 3, a]);
    let pieces = std::array::from_fn(|i| {
        RgbaImage::from_pixel(10, 10, px(if i == 3 || i == 5 || i == 7 { 0 } else { 255 }))
    });
    let mut built = shape::from_nine(SkinWindow::Main, &pieces).unwrap();
    apply(
        SkinWindow::Main,
        &layout,
        &[components],
        &[],
        |_, _| Ok(RgbaImage::from_pixel(10, 2, px(255))),
        &mut built,
    )
    .unwrap();
    assert!(
        built.0.tiles.iter().all(|t| t.decoration.is_none()),
        "{:?}",
        built.0.tiles
    );
}
