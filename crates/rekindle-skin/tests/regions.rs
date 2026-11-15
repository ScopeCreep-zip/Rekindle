//! Validator step 6's text over bitmaps (WCAG F83) and the step-4 flash
//! test (WCAG 2.3.1) (plan E7.7.1 Verify).

use std::collections::BTreeMap;

use rekindle_image::{encode_gif, encode_png, DynamicImage, Frame, Rgba, RgbaImage};
use rekindle_skin::registry::{resolve, Base, ENTRIES};
use rekindle_skin::{images, manifest, package, regions};
use rekindle_types::dto::skin::{SkinError, SkinRule, TokenValue};
use serde_json::{json, Value};

use crate::common::{self, Entry};

/// 8 × 8 of `#101010` with one white pixel at (3, 3).
fn speckled() -> Vec<u8> {
    let image = RgbaImage::from_fn(8, 8, |x, y| {
        if (x, y) == (3, 3) {
            Rgba([255, 255, 255, 255])
        } else {
            Rgba([16, 16, 16, 255])
        }
    });
    encode_png(&DynamicImage::ImageRgba8(image)).unwrap()
}

fn titlebar_manifest(backing: Option<&str>) -> Value {
    let mut m = common::manifest();
    let mut region = json!({ "image": "assets/title.png", "fit": { "type": "stretch" } });
    if let Some(b) = backing {
        region["textBacking"] = json!(b);
    }
    m["regions"] = json!({ "titlebar": region });
    m
}

/// Steps 1, 2 and 4, then F83 with `bg.panel` set to `panel`.
fn run_f83(manifest_json: &Value, panel: [u8; 4]) -> Result<(), SkinError> {
    let bytes = common::zip(&[
        Entry::file("manifest.json", manifest_json.to_string().as_bytes()),
        Entry::file("dark.json", b"{}"),
        Entry::file("assets/title.png", &speckled()),
    ]);
    let package = package::read(&bytes)?;
    let manifest = manifest::parse(&package)?;
    let canonical = images::validate(&package, &manifest)?;
    let [r, g, b, a] = panel;
    let skin = BTreeMap::from([("bg.panel".to_owned(), TokenValue::Color { r, g, b, a })]);
    let resolved = resolve(ENTRIES, Base::Dark, &skin);
    regions::validate(&manifest, &canonical, &resolved)
}

#[test]
fn f83_fails_a_weak_backing_at_the_worst_pixel() {
    let m = titlebar_manifest(Some("bg.panel"));
    let err = run_f83(&m, [0, 0, 0, 0x40]).expect_err("the white pixel shows through");
    assert_eq!(err.entry, "assets/title.png");
    let SkinRule::TextOverBitmap {
        region,
        token,
        backing,
        measured_x100,
        required_x100,
        ..
    } = err.rule
    else {
        panic!("{err:?}");
    };
    assert_eq!(
        (region.as_str(), token.as_str()),
        ("titlebar", "titlebar.text")
    );
    assert_eq!(backing, "bg.panel");
    assert_eq!(required_x100, 450);
    assert!(measured_x100 < 450, "{measured_x100}");
}

#[test]
fn f83_passes_an_opaque_backing() {
    let m = titlebar_manifest(Some("bg.panel"));
    run_f83(&m, [0, 0, 0, 0xff]).unwrap();
}

#[test]
fn f83_needs_a_text_backing_and_a_known_region() {
    let err = run_f83(&titlebar_manifest(None), [0, 0, 0, 0xff]).unwrap_err();
    assert!(
        matches!(err.rule, SkinRule::MissingTextBacking { .. }),
        "{err:?}"
    );
    let mut m = titlebar_manifest(Some("bg.panel"));
    m["regions"] = json!({ "nowhere": m["regions"]["titlebar"].clone() });
    let err = run_f83(&m, [0, 0, 0, 0xff]).unwrap_err();
    assert!(
        matches!(err.rule, SkinRule::UnknownRegion { .. }),
        "{err:?}"
    );
}

/// A GIF alternating black and white, one frame per `delays_cs` entry, its
/// Graphic Control Extension delays (GIF89a §23) patched to `delays_cs`.
fn flashing_gif(delays_cs: &[u16]) -> Vec<u8> {
    let frames = (0..delays_cs.len())
        .map(|i| {
            let v = if i % 2 == 0 { 0 } else { 255 };
            Frame::new(RgbaImage::from_pixel(4, 4, Rgba([v, v, v, 255])))
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

fn run_gif(gif: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, SkinError> {
    let bytes = common::zip(&[
        Entry::file("manifest.json", common::manifest().to_string().as_bytes()),
        Entry::file("dark.json", b"{}"),
        Entry::file("assets/blink.gif", gif),
    ]);
    let package = package::read(&bytes)?;
    let manifest = manifest::parse(&package)?;
    images::validate(&package, &manifest)
}

#[test]
fn a_gif_flashing_four_times_a_second_is_refused() {
    // 120 + 130 ms a cycle: 8 changes, 4 flashes, in every second.
    let err = run_gif(&flashing_gif(&[12, 13, 12, 13])).unwrap_err();
    assert_eq!(err.entry, "assets/blink.gif");
    assert_eq!(err.rule, SkinRule::Flashing { flashes: 4, max: 3 });
}

#[test]
fn a_gif_flashing_three_times_a_second_is_allowed() {
    // 1000 ms for three cycles: 6 changes, 3 flashes.
    run_gif(&flashing_gif(&[17, 16, 17, 17, 16, 17])).unwrap();
}
