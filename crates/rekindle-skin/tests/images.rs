//! Validator step 4: one crafted image per refusal, and the canonical
//! re-encode (plan E7.7.1 Verify).

use std::collections::BTreeMap;

use rekindle_image::{
    decode_limited, encode_gif, encode_png, DecodeLimits, DynamicImage, Frame, Frames, ImageKind,
    Rgba, RgbaImage,
};
use rekindle_skin::limits::{MAX_IMAGE_FRAMES, MAX_IMAGE_PIXELS, MAX_TOTAL_IMAGE_PIXELS};
use rekindle_skin::{images, manifest, package};
use rekindle_types::dto::skin::{SkinError, SkinRule};
use serde_json::{json, Value};

use crate::common::{self, Entry};

fn run_with(
    manifest_json: &Value,
    extra: &[Entry],
) -> Result<BTreeMap<String, Vec<u8>>, SkinError> {
    let mut entries = vec![
        Entry::file("manifest.json", manifest_json.to_string().as_bytes()),
        Entry::file("dark.json", b"{}"),
    ];
    entries.extend_from_slice(extra);
    let bytes = common::zip(&entries);
    let package = package::read(&bytes)?;
    let manifest = manifest::parse(&package)?;
    images::validate(&package, &manifest)
}

fn run(extra: &[Entry]) -> Result<BTreeMap<String, Vec<u8>>, SkinError> {
    run_with(&common::manifest(), extra)
}

fn refused(extra: &[Entry]) -> SkinError {
    run(extra).expect_err("the image must be refused")
}

fn picture(width: u32, height: u32) -> DynamicImage {
    DynamicImage::ImageRgba8(RgbaImage::from_fn(width, height, |x, y| {
        let byte = |v: u32| u8::try_from(v % 256).unwrap_or(0);
        image_rgba([byte(x * 40), byte(y * 40), 7, 255])
    }))
}

fn image_rgba(p: [u8; 4]) -> Rgba<u8> {
    Rgba(p)
}

fn png(width: u32, height: u32) -> Vec<u8> {
    encode_png(&picture(width, height)).unwrap()
}

/// One PNG chunk: length, type, data, CRC-32 (PNG spec §5.3).
fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut body = kind.to_vec();
    body.extend_from_slice(data);
    let mut out = u32::try_from(data.len()).unwrap().to_be_bytes().to_vec();
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc32fast::hash(&body).to_be_bytes());
    out
}

/// `png` with `chunk` inserted right after IHDR (signature 8 + IHDR 25).
fn with_chunk(png: &[u8], chunk: &[u8]) -> Vec<u8> {
    let mut out = png[..33].to_vec();
    out.extend_from_slice(chunk);
    out.extend_from_slice(&png[33..]);
    out
}

/// A PNG whose IHDR says `width` × `height` and that holds no pixel data:
/// only a check made before decoding can pass or refuse it on size.
fn png_header(width: u32, height: u32) -> Vec<u8> {
    let mut ihdr = width.to_be_bytes().to_vec();
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    out.extend(chunk(b"IHDR", &ihdr));
    out.extend(chunk(b"IDAT", &[]));
    out.extend(chunk(b"IEND", &[]));
    out
}

fn gif(frames: usize, width: u32, height: u32) -> Vec<u8> {
    let frames = (0..frames)
        .map(|i| {
            // A blue ramp: distinct frames that never flash (no saturated
            // red, and under 0.10 of luminance), whatever the delays.
            let shade = u8::try_from(i % 256).unwrap();
            Frame::new(RgbaImage::from_pixel(
                width,
                height,
                image_rgba([9, 9, shade, 255]),
            ))
        })
        .collect();
    encode_gif(frames).unwrap()
}

/// `gif` with a comment extension (GIF89a §24) after its colour table.
fn with_comment(gif: &[u8], comment: &[u8]) -> Vec<u8> {
    let packed = gif[10];
    let table = if packed & 0x80 == 0 {
        0
    } else {
        3 << ((packed & 7) + 1)
    };
    let at = 13 + table;
    let mut out = gif[..at].to_vec();
    out.extend_from_slice(&[0x21, 0xfe, u8::try_from(comment.len()).unwrap()]);
    out.extend_from_slice(comment);
    out.push(0);
    out.extend_from_slice(&gif[at..]);
    out
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn a_png_and_a_gif_are_re_encoded_without_their_metadata() {
    let secret = b"Author\0leaked-home-path";
    let tagged_png = with_chunk(&png(5, 4), &chunk(b"tEXt", secret));
    let tagged_gif = with_comment(&gif(3, 4, 4), b"leaked-home-path");
    assert!(contains(&tagged_png, b"leaked-home-path"));
    assert!(contains(&tagged_gif, b"leaked-home-path"));
    let out = run(&[
        Entry::file("assets/frame.png", &tagged_png),
        Entry::file("icons/spinner.gif", &tagged_gif),
    ])
    .unwrap();
    let canonical_png = &out["assets/frame.png"];
    let canonical_gif = &out["icons/spinner.gif"];
    for canonical in [canonical_png, canonical_gif] {
        assert!(
            !contains(canonical, b"leaked-home-path"),
            "metadata survived"
        );
    }
    assert!(!contains(canonical_png, b"tEXt"));
    let limits = DecodeLimits {
        max_width: 64,
        max_height: 64,
        max_pixels: 4096,
        max_alloc: 1 << 20,
        frames: Frames::First,
    };
    let decoded = decode_limited(canonical_png, ImageKind::Png, &limits)
        .unwrap()
        .into_first();
    assert_eq!((decoded.width(), decoded.height()), (5, 4));
    assert_eq!(decoded.to_rgba8(), picture(5, 4).to_rgba8(), "pixels kept");
    assert!(canonical_gif.starts_with(b"GIF89a"));
}

#[test]
fn bytes_that_are_not_the_declared_type_are_refused() {
    let err = refused(&[Entry::file("assets/frame.png", &gif(1, 2, 2))]);
    assert_eq!(err.entry, "assets/frame.png");
    assert_eq!(
        err.rule,
        SkinRule::ImageKindMismatch {
            declared: "image/png".into(),
            found: "image/gif".into(),
        }
    );
    let webp = b"RIFF\x1a\0\0\0WEBPVP8L\x0d\0\0\0\x2f\0\0\0\x10\x07\x10\x11\x11\x88\x88\xfe\x07\0";
    let err = refused(&[Entry::file("assets/frame.gif", webp)]);
    assert_eq!(
        err.rule,
        SkinRule::ImageKindMismatch {
            declared: "image/gif".into(),
            found: "no known image kind".into(),
        }
    );
}

#[test]
fn each_refused_format_is_refused_by_name() {
    let cases: [(&str, &[u8], &str); 8] = [
        ("assets/a.webp", b"RIFF\0\0\0\0WEBPVP8L", "WebP"),
        ("assets/a.jpg", b"\xff\xd8\xff\xe0\0\x10JFIF", "JPEG"),
        ("assets/a.jpeg", b"\xff\xd8\xff\xe0\0\x10JFIF", "JPEG"),
        ("assets/a.bmp", b"BM\0\0\0\0", "BMP"),
        ("assets/a.tiff", b"II*\0", "TIFF"),
        ("assets/a.avif", b"\0\0\0\x1cftypavif", "AVIF"),
        ("icons/a.apng", &png(1, 1), "APNG"),
        (
            "icons/a.svg",
            b"<svg xmlns='http://www.w3.org/2000/svg'/>",
            "SVG",
        ),
    ];
    for (name, bytes, format) in cases {
        let err = refused(&[Entry::file(name, bytes)]);
        assert_eq!(err.entry, name);
        assert_eq!(
            err.rule,
            SkinRule::RefusedImageFormat {
                format: format.into()
            },
            "{name}"
        );
    }
    let err = refused(&[Entry::file("assets/notes.txt", b"hello")]);
    assert!(matches!(err.rule, SkinRule::UnsupportedEntryType { .. }));
}

#[test]
fn an_apng_named_png_is_refused() {
    // acTL: one frame, loop forever (APNG spec §4.1), before the IDAT.
    let actl = chunk(b"acTL", &[0, 0, 0, 1, 0, 0, 0, 0]);
    let err = refused(&[Entry::file(
        "assets/frame.png",
        &with_chunk(&png(2, 2), &actl),
    )]);
    assert_eq!(err.rule, SkinRule::AnimatedPng);
}

#[test]
fn an_image_over_a_side_or_the_pixel_limit_is_refused_from_its_header() {
    let err = refused(&[Entry::file("assets/wide.png", &png_header(4096, 16))]);
    assert_eq!(
        err.rule,
        SkinRule::ImageSide {
            width: 4096,
            height: 16,
            max: 2048
        }
    );
    let err = refused(&[Entry::file("assets/big.png", &png_header(2048, 2000))]);
    assert_eq!(
        err.rule,
        SkinRule::ImagePixels {
            pixels: 2048 * 2000,
            max: MAX_IMAGE_PIXELS
        }
    );
}

#[test]
fn an_animation_over_the_frame_limit_is_refused() {
    let frames = usize::try_from(MAX_IMAGE_FRAMES).unwrap();
    assert!(run(&[Entry::file("icons/ok.gif", &gif(frames, 2, 2))]).is_ok());
    let err = refused(&[Entry::file("icons/long.gif", &gif(frames + 1, 2, 2))]);
    assert_eq!(err.entry, "icons/long.gif");
    assert_eq!(
        err.rule,
        SkinRule::TooManyFrames {
            max: MAX_IMAGE_FRAMES
        }
    );
}

#[test]
fn images_over_the_total_are_refused_before_any_decode() {
    // Sixteen 4 Mpx headers fill the 64 Mpx total; the seventeenth passes
    // it. None holds pixels, so the refusal comes before any decode.
    let header = png_header(2000, 2000);
    let entries: Vec<_> = (0..17)
        .map(|i| Entry::file(&format!("assets/{i:02}.png"), &header))
        .collect();
    let err = refused(&entries);
    assert_eq!(err.entry, "assets/16.png");
    assert_eq!(
        err.rule,
        SkinRule::ImagesTotalPixels {
            pixels: 17 * 4_000_000,
            max: MAX_TOTAL_IMAGE_PIXELS
        }
    );
}

#[test]
fn manifest_references_name_held_files_in_their_directory() {
    let mut m = common::manifest();
    m["icons"] = json!({ "status.online": "assets/online.png" });
    let err = run_with(&m, &[Entry::file("assets/online.png", &png(2, 2))]).unwrap_err();
    assert_eq!(err.entry, "manifest.json");
    assert!(
        matches!(err.rule, SkinRule::BadValue { ref field, .. } if field == "icons.status.online")
    );
    m["icons"] = json!({ "status.online": "icons/online.png" });
    let err = run_with(&m, &[]).unwrap_err();
    assert_eq!(err.entry, "icons/online.png");
    assert!(matches!(err.rule, SkinRule::MissingEntry { .. }));
}
