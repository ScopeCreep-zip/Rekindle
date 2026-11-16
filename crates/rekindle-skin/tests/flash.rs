//! Step 4's flash test, WCAG 2.3.1 in full (plan E7.7.1): the red flash
//! and the small-area clause, measured as the manifest draws each image.

use std::collections::BTreeMap;

use rekindle_image::{encode_gif, Frame, Rgba, RgbaImage};
use rekindle_skin::{images, manifest, package};
use rekindle_types::dto::skin::{SkinError, SkinRule};
use serde_json::{json, Value};

use crate::common::{self, Entry};

/// 4 Hz: 120 + 130 ms a cycle, 4 flashes in every second.
const FOUR_HZ: [u16; 4] = [12, 13, 12, 13];
/// 3 Hz: three cycles in 1000 ms.
const THREE_HZ: [u16; 6] = [17, 16, 17, 17, 16, 17];

const BLACK: [u8; 3] = [16, 16, 16];
const WHITE: [u8; 3] = [255, 255, 255];
const RED: [u8; 3] = [255, 0, 0];
/// The grey of red's relative luminance (0.212 against 0.213): no general
/// flash between them.
const GREY: [u8; 3] = [127, 127, 127];

/// A `width` × `height` GIF whose pixels inside `lit` (x, y, w, h)
/// alternate `a` and `b` frame by frame, the rest staying `BLACK`, with
/// its Graphic Control Extension delays (GIF89a §23) patched to
/// `delays_cs`.
fn gif(
    (width, height): (u32, u32),
    lit: (u32, u32, u32, u32),
    (a, b): ([u8; 3], [u8; 3]),
    delays_cs: &[u16],
) -> Vec<u8> {
    let (lx, ly, lw, lh) = lit;
    let frames = (0..delays_cs.len())
        .map(|i| {
            let on = if i % 2 == 0 { a } else { b };
            Frame::new(RgbaImage::from_fn(width, height, |x, y| {
                let inside = (lx..lx + lw).contains(&x) && (ly..ly + lh).contains(&y);
                let [r, g, b] = if inside { on } else { BLACK };
                Rgba([r, g, b, 255])
            }))
        })
        .collect();
    let mut bytes = encode_gif(frames).unwrap();
    let mut delays = delays_cs.iter();
    let mut i = 0;
    while i + 6 < bytes.len() {
        if bytes[i..i + 3] == [0x21, 0xf9, 0x04] {
            let d = delays.next().expect("one control extension per frame");
            bytes[i + 4..i + 6].copy_from_slice(&d.to_le_bytes());
            i += 8;
        } else {
            i += 1;
        }
    }
    assert!(delays.next().is_none(), "every frame's delay patched");
    bytes
}

/// The window frame drawn from `assets/blink.gif` with `fit`.
fn framed(fit: &Value) -> Value {
    let mut m = common::manifest();
    m["regions"] = json!({ "window.frame": { "image": "assets/blink.gif", "fit": fit } });
    m
}

/// Steps 1, 2 and 4 on a package holding `gif` as `assets/blink.gif`.
fn run(manifest_json: &Value, gif: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, SkinError> {
    let bytes = common::zip(&[
        Entry::file("manifest.json", manifest_json.to_string().as_bytes()),
        Entry::file("dark.json", b"{}"),
        Entry::file("assets/blink.gif", gif),
    ]);
    let package = package::read(&bytes)?;
    let manifest = manifest::parse(&package)?;
    images::validate(&package, &manifest)
}

fn tile() -> Value {
    json!({ "type": "tile" })
}

#[test]
fn a_red_flasher_is_refused_though_its_luminance_holds() {
    let red = gif((4, 4), (0, 0, 4, 4), (RED, GREY), &FOUR_HZ);
    let err = run(&common::manifest(), &red).unwrap_err();
    assert_eq!(err.entry, "assets/blink.gif");
    assert_eq!(err.rule, SkinRule::RedFlashing { flashes: 4, max: 3 });
    // Three red flashes a second are allowed.
    let slow = gif((4, 4), (0, 0, 4, 4), (RED, GREY), &THREE_HZ);
    run(&common::manifest(), &slow).unwrap();
}

#[test]
fn a_small_tiled_flasher_is_allowed_and_a_large_one_refused() {
    // 400 × 300 tiles: one repeat per 341 × 256 field. 147² = 21,609 px
    // flashing is within 25 % of the field (21,824 px); 148² = 21,904 is
    // past it.
    let small = gif((400, 300), (100, 20, 147, 147), (BLACK, WHITE), &FOUR_HZ);
    run(&framed(&tile()), &small).unwrap();
    let large = gif((400, 300), (100, 20, 148, 148), (BLACK, WHITE), &FOUR_HZ);
    let err = run(&framed(&tile()), &large).unwrap_err();
    assert_eq!(err.rule, SkinRule::Flashing { flashes: 4, max: 3 });
    // A red one the same way.
    let red = gif((400, 300), (100, 20, 148, 148), (RED, GREY), &FOUR_HZ);
    let err = run(&framed(&tile()), &red).unwrap_err();
    assert_eq!(err.rule, SkinRule::RedFlashing { flashes: 4, max: 3 });
}

#[test]
fn a_small_tile_repeats_into_a_large_area() {
    // 16 × 16 tiles, 6 × 6 flashing in each: a field meets at most 22 × 17
    // repeats, 13,464 px, allowed; of 8 × 8 it covers 174 × 129 = 22,446
    // px, refused.
    let six = gif((16, 16), (0, 0, 6, 6), (BLACK, WHITE), &FOUR_HZ);
    run(&framed(&tile()), &six).unwrap();
    let eight = gif((16, 16), (0, 0, 8, 8), (BLACK, WHITE), &FOUR_HZ);
    run(&framed(&tile()), &eight).unwrap_err();
}

#[test]
fn a_stretched_or_unplaced_flashing_pixel_is_refused() {
    // Stretched to a window of any size, one pixel can fill the field.
    let one = gif((64, 64), (5, 5, 1, 1), (BLACK, WHITE), &FOUR_HZ);
    let err = run(&framed(&json!({ "type": "stretch" })), &one).unwrap_err();
    assert_eq!(err.rule, SkinRule::Flashing { flashes: 4, max: 3 });
    // An entry no region draws is measured as drawn at any size.
    run(&common::manifest(), &one).unwrap_err();
}

#[test]
fn nine_slice_corners_are_one_to_one_and_edges_stretch() {
    let slice = json!({ "type": "nineSlice",
        "data": { "top": 8, "right": 8, "bottom": 8, "left": 8 } });
    let corner = gif((32, 32), (0, 0, 8, 8), (BLACK, WHITE), &FOUR_HZ);
    run(&framed(&slice), &corner).unwrap();
    let edge = gif((32, 32), (12, 0, 1, 1), (BLACK, WHITE), &FOUR_HZ);
    run(&framed(&slice), &edge).unwrap_err();
}
