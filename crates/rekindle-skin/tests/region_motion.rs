//! `@layer a11y` for skin regions (plan E7.7.2 "Apply"): the static first
//! frame of an animated region GIF, its href in the skin half, its serving
//! by address, and the app half's reduced-motion swap and forced-colors
//! reset in `@layer a11y`.

use std::collections::BTreeMap;

use rekindle_image::{
    decode_limited, encode_gif, encode_png, DecodeLimits, DynamicImage, Frame, Frames, ImageKind,
    Rgba, RgbaImage,
};
use rekindle_skin::regions::css::{app_a11y_css, app_css, skin_css, RegionMap};
use rekindle_skin::regions::still::first_frame;
use rekindle_skin::serve::{body, parse_path, stylesheet, File};
use rekindle_skin::{images, manifest, package, validate};
use serde_json::json;

use crate::common::{self, Entry};

/// `count` distinct frames, a blue ramp that never flashes (WCAG 2.3.1:
/// no saturated red, under 0.10 of luminance).
fn gif(count: u8, width: u32, height: u32) -> Vec<u8> {
    let frames = (0..count)
        .map(|i| {
            Frame::new(RgbaImage::from_pixel(
                width,
                height,
                Rgba([9, 9, i * 40, 255]),
            ))
        })
        .collect();
    encode_gif(frames).unwrap()
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let image = RgbaImage::from_pixel(width, height, Rgba([40, 60, 90, 255]));
    encode_png(&DynamicImage::ImageRgba8(image)).unwrap()
}

fn pixel(png: &[u8]) -> [u8; 4] {
    let limits = DecodeLimits {
        max_width: 64,
        max_height: 64,
        max_pixels: 4096,
        max_alloc: 1 << 20,
        frames: Frames::First,
    };
    let image = decode_limited(png, ImageKind::Png, &limits)
        .unwrap()
        .into_first()
        .into_rgba8();
    image.get_pixel(0, 0).0
}

/// A package whose `titlebar.btn.close` is the animated `assets/normal.gif`
/// with a one-frame `hover` GIF, and whose `scrollbar.thumb` is a PNG.
fn package_bytes() -> Vec<u8> {
    let mut m = common::manifest();
    let nine =
        json!({ "type": "nineSlice", "data": { "top": 2, "right": 3, "bottom": 2, "left": 3 } });
    m["regions"] = json!({
        "titlebar.btn.close": {
            "image": "assets/normal.gif", "fit": nine, "states": { "hover": "assets/hover.gif" }
        },
        "scrollbar.thumb": { "image": "assets/thumb.png", "fit": { "type": "stretch" } }
    });
    common::zip(&[
        Entry::file("manifest.json", m.to_string().as_bytes()),
        Entry::file("dark.json", b"{}"),
        Entry::file("assets/normal.gif", &gif(3, 18, 16)),
        Entry::file("assets/hover.gif", &gif(1, 18, 16)),
        Entry::file("assets/thumb.png", &png(8, 8)),
    ])
}

#[test]
fn a_gif_that_moves_has_its_first_frame_as_a_still_and_one_that_does_not_has_none() {
    let still = first_frame(&gif(3, 4, 4))
        .unwrap()
        .expect("three frames move");
    assert!(still.starts_with(b"\x89PNG\r\n\x1a\n"));
    // The first frame (blue 0), not the second (40) or the last (80).
    assert_eq!(pixel(&still), [9, 9, 0, 255]);
    assert_eq!(first_frame(&gif(1, 4, 4)).unwrap(), None);
    assert!(first_frame(b"GIF89a").is_err());
}

#[test]
fn the_skin_css_addresses_an_animated_region_still_by_its_own_hash() {
    let bytes = package_bytes();
    let p = package::read(&bytes).unwrap();
    let m = manifest::parse(&p).unwrap();
    let canonical = images::validate(&p, &m).unwrap();
    let css = skin_css(&m, &canonical, "skin://localhost/abc").unwrap();
    let still = first_frame(&canonical["assets/normal.gif"])
        .unwrap()
        .unwrap();
    let h = blake3::hash(&still).to_hex();
    assert!(
        css.contains(&format!("    --skin-region-titlebar-btn-close--still: url(\"skin://localhost/abc/{h}.png\") 2 3 2 3 fill / 2px 3px 2px 3px / 0 stretch;\n")),
        "{css}"
    );
    // A one-frame GIF and a PNG do not move: no still.
    assert_eq!(css.matches("--still:").count(), 1, "{css}");
}

#[test]
fn the_still_is_served_by_its_address_like_every_other_image() {
    let c = validate::run(&package_bytes()).unwrap();
    let a = c.address();
    let css = stylesheet(&c.bytes, &a).unwrap().css;
    let decl = css
        .split("--skin-region-titlebar-btn-close--still: url(\"")
        .nth(1)
        .and_then(|r| r.split('"').next())
        .expect("the still's href");
    let name = decl.rsplit_once('/').unwrap().1;
    let path = format!("/{a}/{name}");
    let (_, file) = parse_path(&path).unwrap();
    assert!(matches!(file, File::Image { ext: "png", .. }));
    let served = body(&c.bytes, &a, file).unwrap().expect("the still");
    assert_eq!(served.content_type, "image/png");
    assert_eq!(pixel(&served.bytes), [9, 9, 0, 255]);
    assert_eq!(
        blake3::hash(&served.bytes).to_hex().as_str(),
        name.trim_end_matches(".png")
    );
}

fn small_map() -> BTreeMap<String, RegionMap> {
    let mut map = BTreeMap::new();
    for (slot, el) in [
        ("titlebar.btn.minimize", ".min"),
        ("titlebar.btn.maximize", ".max"),
        ("titlebar.btn.close", ".close"),
        ("scrollbar.track", "::-webkit-scrollbar-track:vertical"),
        ("scrollbar.thumb", "::-webkit-scrollbar-thumb:vertical"),
        (
            "scrollbar.track-horizontal",
            "::-webkit-scrollbar-track:horizontal",
        ),
        (
            "scrollbar.thumb-horizontal",
            "::-webkit-scrollbar-thumb:horizontal",
        ),
        (
            "scrollbar.button-up",
            "::-webkit-scrollbar-button:vertical:decrement",
        ),
        (
            "scrollbar.button-down",
            "::-webkit-scrollbar-button:vertical:increment",
        ),
        (
            "scrollbar.button-left",
            "::-webkit-scrollbar-button:horizontal:decrement",
        ),
        (
            "scrollbar.button-right",
            "::-webkit-scrollbar-button:horizontal:increment",
        ),
        ("scrollbar.corner", "::-webkit-scrollbar-corner"),
        ("button.secondary", ".btn2"),
    ] {
        map.insert(slot.to_owned(), RegionMap::elements(&[el]));
    }
    map.extend(common::surface_entries());
    map
}

#[test]
fn the_app_css_swaps_in_the_still_under_reduced_motion_in_layer_a11y() {
    let components = app_css("h", &small_map()).unwrap();
    assert!(
        !components.contains("@media"),
        "the user's settings are a11y's"
    );
    let css = app_a11y_css("h", &small_map()).unwrap();
    let a11y = css.find("\n@layer a11y {\n").expect("an a11y layer");
    let motion = css
        .find("  @media (prefers-reduced-motion: reduce) {\n")
        .unwrap();
    let forced = css.find("  @media (forced-colors: active) {\n").unwrap();
    // Forced colours last, so it also wins over the swap.
    assert!(a11y < motion && motion < forced);
    let reduced = &css[motion..forced];
    let close =
        "var(--skin-region-titlebar-btn-close--still, var(--skin-region-titlebar-btn-close))";
    assert!(reduced.contains(&format!(
        "    .close {{\n      border-image: {close};\n    }}\n"
    )));
    // A state reads its own still, then its image, then the normal's.
    assert!(reduced.contains(&format!("    .close:hover {{\n      border-image: var(--skin-region-titlebar-btn-close--hover--still, var(--skin-region-titlebar-btn-close--hover, {close}));\n    }}\n")));
    // Every painted rule is swapped.
    assert_eq!(
        reduced.matches("border-image: var(").count(),
        components.matches("border-image: var(").count()
    );
}
