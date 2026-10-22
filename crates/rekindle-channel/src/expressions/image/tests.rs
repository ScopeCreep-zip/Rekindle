use super::*;
use image::codecs::jpeg::JpegEncoder;
use rekindle_image::RgbaImage;

fn picture(width: u32, height: u32) -> DynamicImage {
    let byte = |v: u32| u8::try_from(v % 256).unwrap_or(0);
    let pixels = (0..width * height)
        .flat_map(|i| [byte(i % width), byte(i / width), 7, 255])
        .collect();
    DynamicImage::ImageRgba8(RgbaImage::from_raw(width, height, pixels).unwrap())
}

fn png(width: u32, height: u32) -> Vec<u8> {
    encode_png(&picture(width, height)).unwrap()
}

/// A JPEG carrying an APP1 Exif segment (GPS-like marker text) after SOI.
fn jpeg_with_exif(marker: &[u8]) -> Vec<u8> {
    let mut plain = Vec::new();
    picture(64, 48)
        .to_rgb8()
        .write_with_encoder(JpegEncoder::new(&mut plain))
        .unwrap();
    let mut payload = b"Exif\0\0".to_vec();
    payload.extend_from_slice(marker);
    let len = u16::try_from(payload.len() + 2).unwrap().to_be_bytes();
    let mut out = plain[..2].to_vec();
    out.extend_from_slice(&[0xFF, 0xE1, len[0], len[1]]);
    out.extend_from_slice(&payload);
    out.extend_from_slice(&plain[2..]);
    out
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn an_exif_jpeg_comes_out_as_a_png_without_exif() {
    let marker = b"GPS 51.5007N 0.1246W";
    let source = jpeg_with_exif(marker);
    assert!(contains(&source, b"Exif") && contains(&source, marker));
    let out = prepare_upload(&source, ImageKind::Jpeg, ExpressionPicture::Emoji).unwrap();
    assert_eq!(out.kind, ImageKind::Png);
    assert!(!out.animated());
    assert!(out.bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert!(!contains(&out.bytes, b"Exif"));
    assert!(!contains(&out.bytes, marker));
}

#[test]
fn a_webp_upload_is_refused() {
    let webp = rekindle_image::encode_webp(&picture(16, 16)).unwrap();
    for declared in ImageKind::ALL {
        assert!(prepare_upload(&webp, declared, ExpressionPicture::Emoji).is_err());
        assert!(prepare_upload(&webp, declared, ExpressionPicture::Sticker).is_err());
    }
    assert!(published_media_type(&webp, false, ExpressionPicture::Emoji).is_none());
}

#[test]
fn a_mislabelled_upload_is_refused() {
    assert!(prepare_upload(&png(16, 16), ImageKind::Jpeg, ExpressionPicture::Emoji).is_err());
}

#[test]
fn an_upload_is_scaled_to_the_expression() {
    let out = prepare_upload(&png(600, 300), ImageKind::Png, ExpressionPicture::Sticker).unwrap();
    assert_eq!(
        published_media_type(&out.bytes, false, ExpressionPicture::Sticker),
        Some("image/png")
    );
    let emoji = prepare_upload(&png(600, 300), ImageKind::Png, ExpressionPicture::Emoji).unwrap();
    assert_eq!(
        published_media_type(&emoji.bytes, false, ExpressionPicture::Emoji),
        Some("image/png")
    );
}

#[test]
fn a_gif_stays_animated() {
    let frames = vec![
        Frame::new(picture(200, 200).into_rgba8()),
        Frame::new(picture(200, 200).into_rgba8()),
    ];
    let gif = encode_gif(frames).unwrap();
    let out = prepare_upload(&gif, ImageKind::Gif, ExpressionPicture::Emoji).unwrap();
    assert!(out.animated());
    assert_eq!(out.extension(), "gif");
    assert_eq!(
        published_media_type(&out.bytes, true, ExpressionPicture::Emoji),
        Some("image/gif")
    );
    // An animation is not rendered where a still is expected.
    assert!(published_media_type(&out.bytes, false, ExpressionPicture::Emoji).is_none());
}

#[test]
fn empty_and_oversize_uploads_are_refused() {
    assert!(prepare_upload(&[], ImageKind::Png, ExpressionPicture::Emoji).is_err());
    let oversize = vec![0u8; MAX_STATIC_EMOJI_BYTES + 1];
    assert!(matches!(
        prepare_upload(&oversize, ImageKind::Png, ExpressionPicture::Emoji),
        Err(ChannelError::BodyTooLarge { .. })
    ));
}

#[test]
fn a_malformed_published_picture_is_not_rendered() {
    let mut cut = png(32, 32);
    cut.truncate(40);
    assert!(published_media_type(&cut, false, ExpressionPicture::Emoji).is_none());
    let mut garbage = b"\x89PNG\r\n\x1a\n".to_vec();
    garbage.extend_from_slice(&[0u8; 100]);
    assert!(published_media_type(&garbage, false, ExpressionPicture::Emoji).is_none());
    // Larger than any upload is scaled to.
    assert!(published_media_type(&png(200, 200), false, ExpressionPicture::Emoji).is_none());
    assert_eq!(
        published_media_type(&png(100, 100), false, ExpressionPicture::Emoji),
        Some("image/png")
    );
}
