use super::super::super::tree::parse;
use super::*;

/// Symbiosis's rows `IndentRight="13"` and Infoview tab, Space Cowboy's
/// logo, and an ad slot wholly inside the content.
#[test]
fn indents_are_outsets_and_tiles_above_the_frame_are_decorations() {
    let layout = parse(
        "MainWindow.xml",
        br#"<Tile Name="MainWindow" Component="MainWindow">
<Tile Name="TopRow" Z="1" ResizeX="100" IndentRight="13" Component="Row"/>
<Tile Name="BottomRow" Z="1" JustY="Bottom" ResizeX="100" IndentRight="13" Component="Row"/>
<Tile Name="InfoTab" Z="30" JustX="Right" JustY="Center" Component="Tab"/>
<Tile Name="SCLogo" Z="4" X="0" Y="1" Component="Logo"/>
<Tile Name="Ad" Z="10" JustX="CENTER" Y="12" Component="Ad"/>
<Tile Name="Edge" Z="1" Component="Logo"/></Tile>"#,
    )
    .unwrap();
    let components = parse(
        "Components/MainWindow.xml",
        br#"<Components>
<Component Name="Tab"><Images State="Normal"><Image Type="Middle">tab</Image></Images></Component>
<Component Name="Logo"><Images><Image Type="Middle">logo</Image></Images></Component>
<Component Name="Ad"><Images><Image Type="Middle">ad</Image></Images></Component>
</Components>"#,
    )
    .unwrap();
    let solid = |w, h| RgbaImage::from_pixel(w, h, rekindle_image::Rgba([1, 2, 3, 255]));
    let mut built =
        shape::from_nine(SkinWindow::Main, &std::array::from_fn(|_| solid(10, 10))).unwrap();
    apply(
        SkinWindow::Main,
        &layout,
        &[components],
        |name, _| {
            Ok(match name {
                "tab" => solid(13, 20),
                "logo" => solid(30, 8),
                _ => solid(2, 2),
            })
        },
        &mut built,
    )
    .unwrap();
    assert_eq!((built.0.outsets.left, built.0.outsets.right), (0, 13));
    let got: Vec<_> = built
        .0
        .tiles
        .iter()
        .filter_map(|t| {
            t.decoration
                .map(|z| (t.image.clone(), z, t.x, t.y, t.overhang))
        })
        .collect();
    let tab = (
        "assets/frame/main/decor-infotab.png".to_owned(),
        30,
        TileAxis::End { offset: 0 },
        TileAxis::Centre,
        true,
    );
    let logo = (
        "assets/frame/main/decor-sclogo.png".to_owned(),
        4,
        TileAxis::Start { offset: 0 },
        TileAxis::Start { offset: 1 },
        true,
    );
    assert_eq!(got, vec![tab, logo]);
}

/// Plan SK0-L1: the views' insets are counted from the tile laying the
/// frame's nine-piece set (StealthX: `MainBackground` at X=112, its
/// `ListBackground` 24/23 in, the views 2/2 in that), else from the
/// root less the rows' outsets (Symbiosis: `XfireView` 5/18, rows
/// `IndentRight="13"`); a reference placement reads none.
#[test]
fn views_insets_are_counted_from_the_frame() {
    let nine = parse(
        "Components/MainWindow.xml",
        br#"<Components><Component Name="Bg"><Images>
<Image Type="TopLeft">a</Image><Image Type="Top">a</Image><Image Type="TopRight">a</Image>
<Image Type="Left">a</Image><Image Type="Middle">a</Image><Image Type="Right">a</Image>
<Image Type="BottomLeft">a</Image><Image Type="Bottom">a</Image><Image Type="BottomRight">a</Image>
</Images></Component></Components>"#,
    )
    .unwrap();
    let stealth = parse(
        "MainWindow.xml",
        br#"<Tile Name="W" ResizeX="100" ResizeY="100" Component="MainWindow">
<Tile Name="MainBackground" X="112" Y="0" ResizeX="100" ResizeY="100" Component="Bg">
<Tile Name="List" IndentLeft="24" IndentRight="23" ResizeX="100" ResizeY="100" Component="List">
<Tile Name="TabbedControl" X="2" Y="2" IndentRight="2" ResizeX="100" ResizeY="100" Component="TabbedControl"/>
</Tile></Tile></Tile>"#,
    )
    .unwrap();
    assert_eq!(
        views_insets(&stealth, std::slice::from_ref(&nine)),
        Some((26, 25))
    );
    let symbiosis = parse(
        "MainWindow.xml",
        br#"<Tile Name="MainWindow" Component="MainWindow">
<Tile Name="TopRow" ResizeX="100" IndentRight="13" Component="Row"/>
<Tile Name="BottomRow" JustY="Bottom" ResizeX="100" IndentRight="13" Component="Row"/>
<Tile Name="XfireView" Y="Tabs.Bottom+1" ResizeX="100" ResizeY="100" IndentRight="18" IndentLeft="5" Component="XfireView"/>
</Tile>"#,
    )
    .unwrap();
    assert_eq!(
        views_insets(&symbiosis, std::slice::from_ref(&nine)),
        Some((5, 5))
    );
    let classic = parse(
        "MainWindow.xml",
        br#"<Tile Name="W" Component="MainWindow">
<Tile Name="Inner" X="Edge.RIGHT" ResizeX="100" ResizeY="100" Component="Inner">
<Tile Name="TabbedControl" X="7" IndentRight="6" ResizeX="100" ResizeY="100" Component="TabbedControl"/>
</Tile></Tile>"#,
    )
    .unwrap();
    assert_eq!(views_insets(&classic, &[nine]), None);
}

/// Plan SK2-P5 fix 1: Classic v3's frame rows are its tree's tiles
/// (`MainTitleBar` 14…W−27, `MiddleBevel`, `BaseBoard` 10…W−25), so no
/// nine-piece frame tile also draws their pieces at the window's edges;
/// every image is drawn once, by its tree node in its own box.
#[test]
fn a_frame_read_from_the_trees_rows_is_drawn_only_by_the_tree() {
    let main = parse(
        "MainWindow.xml",
        br#"<Tile Name="W" Component="MainWindow">
<Tile Name="MainTitleBar" Z="4" X="0" Y="0" IndentLeft="14" IndentRight="27" ResizeX="100" Component="MainTitleBar"/>
<Tile Name="MiddleBevel" Z="4" X="9" Y="40" IndentRight="20" IndentBottom="60" ResizeX="100" ResizeY="100" Component="MiddleBevel"/>
<Tile Name="BaseBoard" Z="4" X="0" Y="0" JustY="BOTTOM" IndentLeft="10" IndentRight="25" ResizeX="100" Component="BaseBoard"/>
</Tile>"#,
    )
    .unwrap();
    let components = parse(
        "Components/MainWindow.xml",
        br#"<Components>
<Component Name="MainWindow" MinWidth="200" MinHeight="200"/>
<Component Name="MainTitleBar"><Images State="Normal"><Image Type="topleft">tl-30x47</Image><Image Type="top">t-8x47</Image><Image Type="topright">tr-30x47</Image></Images></Component>
<Component Name="MiddleBevel"><Images><Image Type="left">l-6x8</Image><Image Type="right">r-6x8</Image></Images></Component>
<Component Name="BaseBoard"><Images><Image Type="bottomleft">bl-30x60</Image><Image Type="bottom">b-8x60</Image><Image Type="bottomright">br-30x60</Image></Images></Component>
</Components>"#,
    )
    .unwrap();
    let mut windows = Windows::new();
    main_window(
        &main,
        (std::slice::from_ref(&components), &[]),
        |name, _| {
            let (w, h) = name.rsplit_once('-').unwrap().1.split_once('x').unwrap();
            let (w, h) = (w.parse().unwrap(), h.parse().unwrap());
            Ok(RgbaImage::from_pixel(
                w,
                h,
                rekindle_image::Rgba([9, 9, 9, 255]),
            ))
        },
        &[],
        &mut windows,
    )
    .unwrap();
    let (layout, files) = &windows[&SkinWindow::Main];
    let frame: Vec<&str> = layout.tiles.iter().map(|t| t.image.as_str()).collect();
    assert!(frame.is_empty(), "the frame layer draws {frame:?}");
    let tree = layout.tree.as_ref().unwrap();
    let drawn = tree.root.iter().filter(|n| n.art.is_some()).count();
    assert_eq!(drawn, 3, "each row drawn once by its tree node");
    let pieces = frame_piece_paths(SkinWindow::Main);
    assert!(
        files.keys().all(|k| !pieces.contains(k)),
        "{:?}",
        files.keys()
    );
}

/// Plan SK2 lead decision 27, the fix-1 rule for the row stack: Total
/// Chaos's rows are its tree's tiles, so the stack laid at the window's
/// edges (its `BottomBar` at end 0, 5 px above where the tree lays it)
/// draws none of them again; each row is drawn once, by its node.
#[test]
fn a_row_stack_under_the_tree_is_drawn_only_by_the_tree() {
    let main = parse(
        "mainwindow.xml",
        br#"<Tile Name="W" ResizeX="100" ResizeY="100" Component="MainWindow">
<Tile Name="MainTitleBar" X="0" OutdentLeft="1" Y="0" ResizeX="100" ResizeY="0" Component="MainTitleBar"/>
<Tile Name="Content" X="0" OutdentLeft="1" Y="MainTitleBar.BOTTOM" IndentBottom="61" ResizeY="100" ResizeX="100" Component="Content"/>
<Tile Name="BottomBar" X="0" OutdentLeft="1" Y="Content.BOTTOM" ResizeX="100" Component="BottomBar"/>
</Tile>"#,
    )
    .unwrap();
    let components = parse(
        "components.xml",
        br#"<Components>
<Component Name="MainWindow" MinWidth="200" MinHeight="200"/>
<Component Name="MainTitleBar"><Images State="Normal"><Image Type="left">tl-75x52</Image><Image Type="middle">tm-8x52</Image><Image Type="right">tr-75x52</Image></Images></Component>
<Component Name="Content"><Images State="Normal"><Image Type="left">cl-59x37</Image><Image Type="middle">cm-9x9</Image><Image Type="right">cr-59x37</Image></Images></Component>
<Component Name="BottomBar"><Images State="Normal"><Image Type="left">bl-61x66</Image><Image Type="middle">bm-37x66</Image><Image Type="right">br-61x66</Image></Images></Component>
</Components>"#,
    )
    .unwrap();
    let mut windows = Windows::new();
    main_window(
        &main,
        (std::slice::from_ref(&components), &[]),
        |name, _| {
            let (w, h) = name.rsplit_once('-').unwrap().1.split_once('x').unwrap();
            let (w, h) = (w.parse().unwrap(), h.parse().unwrap());
            Ok(RgbaImage::from_pixel(
                w,
                h,
                rekindle_image::Rgba([9, 9, 9, 255]),
            ))
        },
        &[],
        &mut windows,
    )
    .unwrap();
    let (layout, files) = &windows[&SkinWindow::Main];
    let frame: Vec<&str> = layout.tiles.iter().map(|t| t.image.as_str()).collect();
    assert!(frame.is_empty(), "the frame layer draws {frame:?}");
    let rows = stack::row_paths(SkinWindow::Main, 3);
    assert!(
        files.keys().all(|k| !rows.contains(k)),
        "{:?}",
        files.keys()
    );
    assert_eq!(layout.tree.as_ref().unwrap().outsets.bottom, 0);
}
