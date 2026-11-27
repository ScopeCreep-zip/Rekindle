use super::*;

fn striped(w: u32) -> RgbaImage {
    RgbaImage::from_fn(w, 2, |x, _| {
        rekindle_image::Rgba([u8::try_from(x).unwrap(), 0, 0, 255])
    })
}

#[test]
fn sides_wider_than_the_views_inset_keep_their_outer_columns() {
    let (mut l, mut r) = (striped(59), striped(59));
    to_views_edge(&mut l, &mut r, (50, 8));
    assert_eq!((l.width(), r.width()), (50, 8));
    assert_eq!(l.get_pixel(0, 0)[0], 0);
    assert_eq!(r.get_pixel(7, 0)[0], 58);
}

/// Plan SK0-L1 (Alloy, Combat, XGlow): rows taller than the views'
/// inset keep their outer rows, the middle row and shorter pieces stay
/// whole, and the built content is the views' rectangle.
#[test]
fn rows_taller_than_the_views_inset_keep_their_outer_rows() {
    let rows = |h: u32| {
        RgbaImage::from_fn(4, h, |_, y| {
            rekindle_image::Rgba([u8::try_from(y).unwrap(), 0, 0, 255])
        })
    };
    let mut nine: [RgbaImage; 9] = std::array::from_fn(|i| match i {
        0..=2 => rows(30),
        6..=8 => rows(20),
        _ => rows(5),
    });
    nine[1] = rows(10);
    to_views_rows(&mut nine, (16, 7));
    let heights: Vec<u32> = nine.iter().map(RgbaImage::height).collect();
    assert_eq!(heights, vec![16, 10, 16, 5, 5, 5, 7, 7, 7]);
    assert_eq!(nine[0].get_pixel(0, 15)[0], 15);
    assert_eq!(nine[8].get_pixel(0, 0)[0], 13);
    let built = shape::from_nine(SkinWindow::Main, &nine).unwrap();
    assert_eq!((built.0.content.top, built.0.content.bottom), (16, 7));
}

#[test]
fn sides_narrower_than_the_views_inset_or_no_inset_stay_whole() {
    let (mut l, mut r) = (striped(16), striped(20));
    to_views_edge(&mut l, &mut r, (27, 0));
    assert_eq!((l.width(), r.width()), (16, 20));
}

/// Plan SK0-L1 (Space Cowboy, sides narrower than the views): the
/// content is the views' rectangle and the band is the centre art; a
/// centre with a transparent pixel lays no band.
#[test]
fn a_narrow_sides_band_to_the_views_is_the_centre_fill() {
    let px = |a| rekindle_image::Rgba([9, 9, 9, a]);
    let nine: [RgbaImage; 9] = std::array::from_fn(|i| {
        if i == 4 {
            RgbaImage::from_fn(3, 2, |x, _| {
                rekindle_image::Rgba([u8::try_from(x).unwrap(), 0, 0, 255])
            })
        } else {
            RgbaImage::from_pixel(4, 4, px(255))
        }
    });
    let mut built = shape::from_nine(SkinWindow::Main, &nine).unwrap();
    fill_band(SkinWindow::Main, &mut built, &nine[4], (9, 4)).unwrap();
    assert_eq!((built.0.content.left, built.0.content.right), (9, 4));
    assert!(built.0.min_width >= 13);
    let fills: Vec<_> = built
        .0
        .tiles
        .iter()
        .filter(|t| t.fill)
        .map(|t| (t.width, t.height, t.x, t.y, t.overhang))
        .collect();
    let y = TileAxis::Repeat { start: 4, end: 4 };
    assert_eq!(fills, vec![(5, 2, TileAxis::Start { offset: 4 }, y, false)]);
    let mut clear = shape::from_nine(SkinWindow::Main, &nine).unwrap();
    let mut centre = nine[4].clone();
    centre.put_pixel(1, 1, px(0));
    fill_band(SkinWindow::Main, &mut clear, &centre, (9, 4)).unwrap();
    assert_eq!(clear.0.content.left, 4);
    assert!(!clear.0.tiles.iter().any(|t| t.fill));
}
