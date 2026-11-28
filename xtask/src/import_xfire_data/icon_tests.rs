//! Synthetic `.ICO` fixtures only: every image is built here.

use super::*;
use rekindle_image::{decode_limited, DecodeLimits, Frames, ImageKind};

/// A BITMAPINFOHEADER DIB of `w`×`w` at `bpp`, palette `palette` (BGRX),
/// XOR rows `xor` and AND rows `and`, both bottom-up and already padded.
fn dib(w: u32, bpp: u16, palette: &[[u8; 4]], xor: &[u8], and: &[u8]) -> Vec<u8> {
    let mut d = Vec::new();
    d.extend_from_slice(&40u32.to_le_bytes());
    d.extend_from_slice(&w.to_le_bytes());
    d.extend_from_slice(&(w * 2).to_le_bytes());
    d.extend_from_slice(&1u16.to_le_bytes());
    d.extend_from_slice(&bpp.to_le_bytes());
    d.extend_from_slice(&[0; 16]); // compression, image size, x and y ppm
    d.extend_from_slice(&u32::try_from(palette.len()).unwrap().to_le_bytes());
    d.extend_from_slice(&[0; 4]); // important colours
    for c in palette {
        d.extend_from_slice(c);
    }
    d.extend_from_slice(xor);
    d.extend_from_slice(and);
    d
}

/// An `.ICO` holding `images`.
fn ico(images: &[Vec<u8>]) -> Vec<u8> {
    let mut out = vec![0, 0, 1, 0];
    out.extend_from_slice(&u16::try_from(images.len()).unwrap().to_le_bytes());
    let mut offset = 6 + 16 * images.len();
    for img in images {
        out.extend_from_slice(&[0; 8]);
        out.extend_from_slice(&u32::try_from(img.len()).unwrap().to_le_bytes());
        out.extend_from_slice(&u32::try_from(offset).unwrap().to_le_bytes());
        offset += img.len();
    }
    for img in images {
        out.extend_from_slice(img);
    }
    out
}

fn pixels(png: &[u8]) -> RgbaImage {
    let limits = DecodeLimits {
        max_width: 256,
        max_height: 256,
        max_pixels: 65_536,
        max_alloc: 1 << 20,
        frames: Frames::First,
    };
    decode_limited(png, ImageKind::Png, &limits)
        .unwrap()
        .into_first()
        .to_rgba8()
}

/// The AND mask of a 2×2 image hiding its top-left pixel: rows of 4 bytes,
/// bottom row first; bit 7 of a row's byte 0 is pixel 0.
const MASK_TOP_LEFT: [u8; 8] = [0, 0, 0, 0, 0b1000_0000, 0, 0, 0];

/// A 2×2 8-bit DIB: palette 0 red, 1 blue. The bottom row (stored first)
/// is red, blue; the top row blue, red.
fn two_by_two_8bit() -> Vec<u8> {
    let palette = [[0, 0, 255, 0], [255, 0, 0, 0]];
    let xor = [0, 1, 0, 0, 1, 0, 0, 0];
    dib(2, 8, &palette, &xor, &MASK_TOP_LEFT)
}

#[test]
fn an_indexed_dib_takes_its_alpha_from_the_and_mask() {
    let img = pixels(&to_png("t", &ico(&[two_by_two_8bit()])).unwrap());
    assert_eq!(
        img.get_pixel(0, 0).0,
        [0, 0, 255, 0],
        "top-left blue, masked"
    );
    assert_eq!(img.get_pixel(1, 0).0, [255, 0, 0, 255], "top-right red");
    assert_eq!(img.get_pixel(0, 1).0, [255, 0, 0, 255], "bottom-left red");
    assert_eq!(img.get_pixel(1, 1).0, [0, 0, 255, 255], "bottom-right blue");
}

#[test]
fn a_32_bit_dib_keeps_its_own_alpha_and_falls_back_to_the_mask_without_one() {
    let with_alpha = [
        10, 20, 30, 128, 10, 20, 30, 255, 10, 20, 30, 255, 10, 20, 30, 0,
    ];
    let img = pixels(&to_png("t", &ico(&[dib(2, 32, &[], &with_alpha, &MASK_TOP_LEFT)])).unwrap());
    assert_eq!(img.get_pixel(0, 1).0, [30, 20, 10, 128], "own alpha");
    assert_eq!(
        img.get_pixel(0, 0).0[3],
        255,
        "own alpha wins over the mask"
    );
    let no_alpha = [10, 20, 30, 0, 10, 20, 30, 0, 10, 20, 30, 0, 10, 20, 30, 0];
    let img = pixels(&to_png("t", &ico(&[dib(2, 32, &[], &no_alpha, &MASK_TOP_LEFT)])).unwrap());
    assert_eq!(img.get_pixel(0, 0).0[3], 0, "the mask hides top-left");
    assert_eq!(img.get_pixel(1, 1).0[3], 255);
}

#[test]
fn the_largest_image_is_kept_and_a_png_is_copied() {
    let png = encode_png(&DynamicImage::ImageRgba8(RgbaImage::new(4, 4))).unwrap();
    let out = to_png("t", &ico(&[two_by_two_8bit(), png.clone()])).unwrap();
    assert_eq!(out, png, "the 4x4 PNG outranks the 2x2 DIB and is copied");
    let out = to_png("t", &ico(&[png.clone(), two_by_two_8bit()])).unwrap();
    assert_eq!(out, png);
}

#[test]
fn out_of_bounds_icons_are_refused() {
    let mut wide = two_by_two_8bit();
    wide[4..8].copy_from_slice(&300u32.to_le_bytes());
    assert!(to_png("t", &ico(&[wide])).is_err(), "width past 256");
    let short = two_by_two_8bit()[..50].to_vec();
    assert!(to_png("t", &ico(&[short])).is_err(), "truncated pixels");
    let mut past = ico(&[two_by_two_8bit()]);
    past[18..22].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(to_png("t", &past).is_err(), "payload offset past the file");
    let mut rle = two_by_two_8bit();
    rle[16] = 1;
    assert!(to_png("t", &ico(&[rle])).is_err(), "BI_RLE8");
    assert!(
        to_png("t", &[0, 0, 2, 0, 1, 0]).is_err(),
        "a cursor, not an icon"
    );
}
