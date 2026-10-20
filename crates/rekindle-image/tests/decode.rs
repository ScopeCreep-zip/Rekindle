//! `decode_limited` against the rules it enforces (plan E7.11 Verify).

use std::time::Duration;

use rekindle_image::{
    decode_limited, encode_gif, encode_png, encode_webp, fit_within, DecodeLimits, Decoded,
    DynamicImage, Frame, Frames, ImageError, ImageKind, RgbaImage, TooLarge,
};

const LIMITS: DecodeLimits = DecodeLimits {
    max_width: 4096,
    max_height: 4096,
    max_pixels: 4_000_000,
    max_alloc: 64 * 1024 * 1024,
    frames: Frames::First,
};

fn picture(width: u32, height: u32) -> DynamicImage {
    DynamicImage::ImageRgba8(RgbaImage::from_fn(width, height, |x, y| {
        let byte = |v: u32| u8::try_from(v % 256).unwrap_or(0);
        image::Rgba([byte(x), byte(y), 7, 255])
    }))
}

/// The CRC-32 a PNG chunk ends with (PNG spec §5.5, ISO 3309).
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// One PNG chunk: length, type, data, CRC.
fn chunk(png: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    let mut body = kind.to_vec();
    body.extend_from_slice(data);
    png.extend_from_slice(&u32::try_from(data.len()).unwrap().to_be_bytes());
    png.extend_from_slice(&body);
    png.extend_from_slice(&crc32(&body).to_be_bytes());
}

/// A PNG whose IHDR says `width` × `height`, with an empty IDAT: no pixel
/// data, so a decoder that allocated from the header would be caught.
fn png_header(width: u32, height: u32) -> Vec<u8> {
    let mut ihdr = width.to_be_bytes().to_vec();
    ihdr.extend_from_slice(&height.to_be_bytes());
    // Bit depth 8, colour type 6 (RGBA), deflate, filter 0, no interlace.
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(&mut png, b"IDAT", &[]);
    chunk(&mut png, b"IEND", &[]);
    png
}

#[test]
fn a_png_declared_as_jpeg_is_a_kind_mismatch() {
    let png = encode_png(&picture(8, 8)).unwrap();
    let refused = decode_limited(&png, ImageKind::Jpeg, &LIMITS).err();
    assert!(
        matches!(
            refused,
            Some(ImageError::KindMismatch {
                declared: ImageKind::Jpeg,
                found: Some(ImageKind::Png),
            })
        ),
        "{refused:?}"
    );
    let refused = decode_limited(b"not an image", ImageKind::Png, &LIMITS).err();
    assert!(matches!(
        refused,
        Some(ImageError::KindMismatch { found: None, .. })
    ));
}

#[test]
fn a_huge_header_is_refused_before_allocation() {
    let header = png_header(30_000, 30_000);
    let refused = decode_limited(&header, ImageKind::Png, &LIMITS).err();
    assert!(
        matches!(
            refused,
            Some(ImageError::TooLarge(TooLarge::Side {
                width: 30_000,
                height: 30_000,
                ..
            }))
        ),
        "{refused:?}"
    );
    // Within the sides but over the pixel count: 3000 × 3000 = 9 Mpx.
    let refused = decode_limited(&png_header(3000, 3000), ImageKind::Png, &LIMITS).err();
    assert!(
        matches!(
            refused,
            Some(ImageError::TooLarge(TooLarge::Pixels {
                pixels: 9_000_000,
                ..
            }))
        ),
        "{refused:?}"
    );
}

#[test]
fn a_decode_over_max_alloc_is_too_large() {
    let png = encode_png(&picture(256, 256)).unwrap();
    let tight = DecodeLimits {
        max_alloc: 1024,
        ..LIMITS
    };
    let refused = decode_limited(&png, ImageKind::Png, &tight).err();
    assert!(
        matches!(
            refused,
            Some(ImageError::TooLarge(TooLarge::Allocation { .. }))
        ),
        "{refused:?}"
    );
}

#[test]
fn a_normal_image_becomes_a_small_webp() {
    let png = encode_png(&picture(640, 480)).unwrap();
    let still = decode_limited(&png, ImageKind::Png, &LIMITS)
        .unwrap()
        .into_first();
    assert_eq!((still.width(), still.height()), (640, 480));
    let webp = encode_webp(&fit_within(still, 128)).unwrap();
    let back = image::load_from_memory_with_format(&webp, image::ImageFormat::WebP).unwrap();
    assert_eq!((back.width(), back.height()), (128, 96));
}

#[test]
fn a_gif_is_its_first_frame_or_every_frame_within_the_count() {
    let delay = image::Delay::from_saturating_duration(Duration::from_millis(100));
    let frames = (0..3)
        .map(|i| {
            let mut buffer = RgbaImage::new(16, 16);
            buffer.put_pixel(i, i, image::Rgba([255, 0, 0, 255]));
            Frame::from_parts(buffer, 0, 0, delay)
        })
        .collect();
    let gif = encode_gif(frames).unwrap();

    let first = decode_limited(&gif, ImageKind::Gif, &LIMITS).unwrap();
    assert!(matches!(first, Decoded::Still(_)));

    let every = DecodeLimits {
        frames: Frames::AtMost(3),
        ..LIMITS
    };
    match decode_limited(&gif, ImageKind::Gif, &every).unwrap() {
        Decoded::Animated { first, rest } => {
            assert_eq!(rest.len(), 2);
            assert_eq!(first.buffer().dimensions(), (16, 16));
            assert_eq!(first.delay(), delay);
        }
        Decoded::Still(_) => panic!("an animation read with AtMost is animated"),
    }

    let two = DecodeLimits {
        frames: Frames::AtMost(2),
        ..LIMITS
    };
    let refused = decode_limited(&gif, ImageKind::Gif, &two).err();
    assert!(
        matches!(
            refused,
            Some(ImageError::TooLarge(TooLarge::Frames { max_frames: 2 }))
        ),
        "{refused:?}"
    );
}

#[test]
fn a_png_and_a_jpeg_decode_by_their_declared_kind() {
    let png = encode_png(&picture(32, 20)).unwrap();
    let decoded = decode_limited(&png, ImageKind::Png, &LIMITS)
        .unwrap()
        .into_first();
    assert_eq!((decoded.width(), decoded.height()), (32, 20));

    let mut jpeg = Vec::new();
    DynamicImage::ImageRgb8(picture(32, 20).to_rgb8())
        .write_to(
            &mut std::io::Cursor::new(&mut jpeg),
            image::ImageFormat::Jpeg,
        )
        .unwrap();
    let decoded = decode_limited(&jpeg, ImageKind::Jpeg, &LIMITS)
        .unwrap()
        .into_first();
    assert_eq!((decoded.width(), decoded.height()), (32, 20));
    assert!(matches!(
        decode_limited(&jpeg, ImageKind::Png, &LIMITS),
        Err(ImageError::KindMismatch {
            found: Some(ImageKind::Jpeg),
            ..
        })
    ));
}

/// A camera-like photo: smooth light falling across the frame plus sensor
/// noise (a fixed LCG), so the JPEG has real high-frequency content.
fn photo(width: u32, height: u32) -> image::RgbImage {
    let mut state = 0x2545_f491_u32;
    image::RgbImage::from_fn(width, height, |x, y| {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let noise = (state >> 24) % 24;
        let channel = |base: u32| u8::try_from((base + noise).min(255)).unwrap_or(255);
        image::Rgb([
            channel(x * 200 / width),
            channel(y * 180 / height + 20),
            channel((x + y) * 120 / (width + height) + 60),
        ])
    })
}

/// The avatar path for a photo (plan E7.11 Verify: "a normal photo still
/// becomes a ≤ `AVATAR_MAX_DIM` WebP"): a 3 Mpx 4:3 JPEG at quality 85,
/// declared as a JPEG, decodes under the limits and becomes a 128 px WebP
/// that decodes.
#[test]
fn a_jpeg_photo_becomes_a_small_webp() {
    const AVATAR_MAX_DIM: u32 = 128;
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 85)
        .encode_image(&photo(2016, 1512))
        .unwrap();
    assert_eq!(&jpeg[..3], [0xff, 0xd8, 0xff]);

    let still = decode_limited(&jpeg, ImageKind::Jpeg, &LIMITS)
        .unwrap()
        .into_first();
    assert_eq!((still.width(), still.height()), (2016, 1512));
    let webp = encode_webp(&fit_within(still, AVATAR_MAX_DIM)).unwrap();
    let back = image::load_from_memory_with_format(&webp, image::ImageFormat::WebP).unwrap();
    assert!(back.width() <= AVATAR_MAX_DIM && back.height() <= AVATAR_MAX_DIM);
    assert_eq!((back.width(), back.height()), (128, 96));
}
