use rekindle_image::{Rgba, RgbaImage};

use super::icon_box;

/// A 46 x 48 window ground (`#151515`) with a plate graded top to
/// bottom inside a dark rim, as Symbiosis's tab art.
fn plate() -> RgbaImage {
    RgbaImage::from_fn(46, 48, |x, y| {
        if !(2..44).contains(&x) || !(9..38).contains(&y) {
            Rgba([0x15, 0x15, 0x15, 255])
        } else if x == 2 || x == 43 || y == 9 || y == 37 {
            Rgba([9, 9, 9, 255])
        } else {
            let v = u8::try_from(0x59 - (y - 10) / 2).unwrap();
            Rgba([v, v - 4, v - 8, 255])
        }
    })
}

#[test]
fn the_icon_on_a_graded_plate_is_its_box() {
    let mut art = plate();
    // Wider than the plate left beside it in its rows.
    for y in 20..26 {
        for x in 13..33 {
            art.put_pixel(x, y, Rgba([255, 255, 255, 255]));
        }
    }
    assert_eq!(icon_box(&art), Some([13, 20, 20, 6]));
}

#[test]
fn a_plain_plate_or_an_icon_on_the_frame_has_no_cover() {
    assert_eq!(icon_box(&plate()), None);
    let mut art = plate();
    for x in 0..46 {
        art.put_pixel(x, 24, Rgba([255, 255, 255, 255]));
    }
    assert_eq!(icon_box(&art), None);
}
