use super::*;

/// A lossless WebP's size, from its VP8L header (RFC 9649 §3: the
/// 0x2f signature, then 14 bits each of width − 1 and height − 1).
fn webp_size(webp: &[u8]) -> (u32, u32) {
    assert_eq!(&webp[0..4], b"RIFF");
    assert_eq!(&webp[8..16], b"WEBPVP8L");
    assert_eq!(webp[20], 0x2f);
    let bits = u32::from_le_bytes([webp[21], webp[22], webp[23], webp[24]]);
    ((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1)
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let byte = |v: u32| u8::try_from(v % 256).unwrap_or(0);
    let pixels = (0..width * height)
        .flat_map(|i| [byte(i % width), byte(i / width), 7, 255])
        .collect();
    let picture = rekindle_image::RgbaImage::from_raw(width, height, pixels).unwrap();
    rekindle_image::encode_png(&rekindle_image::DynamicImage::ImageRgba8(picture)).unwrap()
}

/// A normal picture becomes a small WebP that fits the profile.
#[test]
fn an_avatar_is_a_small_webp() {
    let webp = compress_avatar(&png(640, 480), ImageKind::Png).unwrap();
    assert!(webp.len() <= PROFILE_SUBKEY_CAP);
    let (width, height) = webp_size(&webp);
    assert!(width <= AVATAR_MAX_DIM && height <= AVATAR_MAX_DIM);
    assert_eq!((width, height), (128, 96));
    assert!(compress_avatar(b"not an image", ImageKind::Png).is_err());
}

/// The picker's kind is the only decoder: a PNG sent as a JPEG is
/// refused, not sniffed.
#[test]
fn a_png_declared_as_jpeg_is_refused() {
    let refused = compress_avatar(&png(8, 8), ImageKind::Jpeg).unwrap_err();
    assert!(refused.contains("declared image/jpeg"), "{refused}");
}
