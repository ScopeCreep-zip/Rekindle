use rekindle_image::Rgba;

use super::*;

#[test]
fn key_and_shift() {
    let mut i = RgbaImage::from_pixel(2, 1, Rgba([0xff, 0, 0xff, 0xff]));
    i.put_pixel(1, 0, Rgba([100, 250, 10, 0xff]));
    colour_key(&mut i);
    assert_eq!(i.get_pixel(0, 0).0, [0, 0, 0, 0]);
    shift(&mut i, [30, 30, -30]);
    assert_eq!(i.get_pixel(1, 0).0, [130, 255, 0, 0xff]);
    assert_eq!(i.get_pixel(0, 0).0, [0, 0, 0, 0]);
}

#[test]
fn nine_pieces_compose() {
    let p = |w, h, v| RgbaImage::from_pixel(w, h, Rgba([v, v, v, 255]));
    let pieces = [
        p(6, 4, 1),
        p(4, 4, 2),
        p(6, 4, 3),
        p(6, 4, 4),
        p(24, 24, 5),
        p(6, 4, 6),
        p(6, 4, 7),
        p(4, 4, 8),
        p(6, 4, 9),
    ];
    let (img, insets) = compose("f", &pieces).unwrap();
    assert_eq!((img.width(), img.height()), (36, 32));
    assert_eq!(
        (insets.top, insets.right, insets.bottom, insets.left),
        (4, 6, 4, 6)
    );
    assert_eq!(img.get_pixel(20, 2).0[0], 2);
    assert_eq!(img.get_pixel(2, 20).0[0], 4);
    assert_eq!(img.get_pixel(35, 31).0[0], 9);
}
