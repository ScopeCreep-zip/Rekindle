//! The community picture limits (plan E7.11): what an upload may be.

use super::*;

fn png(width: u32, height: u32) -> Vec<u8> {
    let pixels = vec![9u8; (width * height * 4) as usize];
    let picture = rekindle_image::RgbaImage::from_raw(width, height, pixels).unwrap();
    rekindle_image::encode_png(&rekindle_image::DynamicImage::ImageRgba8(picture)).unwrap()
}

/// A picture is cut to its slot and comes out a WebP.
#[test]
fn a_banner_is_a_webp_within_its_box() {
    let webp = compress(&png(1200, 200), ImageKind::Png, PictureSlot::Banner).unwrap();
    assert!(is_webp(&webp));
    let avatar = compress(&png(300, 300), ImageKind::Png, PictureSlot::Avatar).unwrap();
    assert!(is_webp(&avatar));
}

/// A WebP upload is refused: inputs are PNG, JPEG or GIF only, and the
/// declared kind is the only decoder.
#[test]
fn a_webp_upload_is_refused() {
    let webp = compress(&png(16, 16), ImageKind::Png, PictureSlot::Avatar).unwrap();
    for kind in ImageKind::ALL {
        assert!(compress(&webp, kind, PictureSlot::Avatar).is_err(), "{kind:?}");
    }
    assert!(compress(&png(8, 8), ImageKind::Jpeg, PictureSlot::Avatar).is_err());
}

/// A picture past the side limit is refused from its header.
#[test]
fn an_oversized_picture_is_refused() {
    let refused = compress(&png(8193, 1), ImageKind::Png, PictureSlot::Avatar).unwrap_err();
    assert!(refused.contains("8193"), "{refused}");
}

/// Only a 64-digit lowercase hex hash names a cached picture.
#[test]
fn only_a_hash_names_a_picture() {
    assert!(is_picture_hash(&"a".repeat(64)));
    assert!(!is_picture_hash(&"A".repeat(64)));
    assert!(!is_picture_hash("../../etc/passwd"));
    assert!(!is_picture_hash(&"a".repeat(63)));
}
