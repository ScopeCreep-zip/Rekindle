use std::collections::BTreeMap;

use rekindle_image::{encode_gif, encode_png, DynamicImage, Frame, Rgba, RgbaImage};
use rekindle_types::dto::skin::{SkinManifest, SkinRule};

use super::validate;
use crate::registry::{resolve, Base, ENTRIES};
use crate::{builtin, manifest, package};

/// The token look's manifest (Symbiosis) with one status icon.
fn with_icon(id: &str, path: &str) -> SkinManifest {
    let (_, b) = builtin::find(builtin::DEFAULT).unwrap();
    let mut m = manifest::parse(&package::read(b.bytes).unwrap()).unwrap();
    m.icons = BTreeMap::from([(id.to_owned(), path.to_owned())]);
    m
}

fn png(w: u32, h: u32, px: [u8; 4]) -> Vec<u8> {
    encode_png(&DynamicImage::ImageRgba8(RgbaImage::from_pixel(
        w,
        h,
        Rgba(px),
    )))
    .unwrap()
}

fn run(id: &str, path: &str, bytes: Vec<u8>, base: Base) -> Result<(), SkinRule> {
    let m = with_icon(id, path);
    let images = BTreeMap::from([(path.to_owned(), bytes)]);
    let resolved = resolve(ENTRIES, base, &BTreeMap::new());
    validate(&m, &images, base, &resolved).map_err(|e| e.rule)
}

const GREEN: [u8; 4] = [0x2e, 0xd5, 0x73, 255];

#[test]
fn a_bright_square_icon_passes_on_the_dark_base() {
    for side in super::ICON_SIDES {
        let r = run(
            "status.online",
            "icons/status.online.png",
            png(side, side, GREEN),
            Base::Dark,
        );
        assert_eq!(r, Ok(()), "side {side}");
    }
}

#[test]
fn an_icon_that_is_not_square_or_off_the_sizes_is_refused() {
    for (w, h) in [(12, 16), (13, 13), (64, 64)] {
        let r = run(
            "status.away",
            "icons/status.away.png",
            png(w, h, GREEN),
            Base::Dark,
        );
        assert_eq!(
            r,
            Err(SkinRule::IconSize {
                icon: "status.away".to_owned(),
                width: w,
                height: h
            })
        );
    }
}

#[test]
fn a_dark_fill_on_the_dark_rows_fails_s3() {
    let r = run(
        "status.busy",
        "icons/status.busy.png",
        png(16, 16, [0x18, 0x18, 0x18, 255]),
        Base::Dark,
    );
    assert!(
        matches!(&r, Err(SkinRule::Contrast { rule, token, .. }) if rule == "S3" && token == "icons.status.busy"),
        "{r:?}"
    );
}

#[test]
fn a_transparent_hole_in_the_shown_disc_shows_the_ground_and_fails() {
    let mut img = RgbaImage::from_pixel(12, 12, Rgba(GREEN));
    img.put_pixel(6, 6, Rgba([0, 0, 0, 0]));
    let bytes = encode_png(&DynamicImage::ImageRgba8(img)).unwrap();
    let r = run(
        "status.ingame",
        "icons/status.ingame.png",
        bytes,
        Base::Dark,
    );
    assert!(matches!(r, Err(SkinRule::Contrast { .. })), "{r:?}");
}

#[test]
fn the_ring_measures_only_its_band_and_skips_the_cutout() {
    // A dark centre the ring never shows; a bright band it does.
    let mut img = RgbaImage::from_pixel(12, 12, Rgba(GREEN));
    for (x, y) in [(5, 5), (6, 6), (5, 6), (6, 5)] {
        img.put_pixel(x, y, Rgba([0x0a, 0x0a, 0x0a, 255]));
    }
    let bytes = encode_png(&DynamicImage::ImageRgba8(img)).unwrap();
    let r = run(
        "status.invisible",
        "icons/status.invisible.png",
        bytes.clone(),
        Base::Dark,
    );
    assert_eq!(r, Ok(()));
    // The disc shows that centre: the same fill fails as online.
    assert!(run(
        "status.online",
        "icons/status.online.png",
        bytes,
        Base::Dark
    )
    .is_err());
}

#[test]
fn every_frame_of_an_animated_icon_is_measured() {
    let lit = RgbaImage::from_pixel(12, 12, Rgba(GREEN));
    let dark = RgbaImage::from_pixel(12, 12, Rgba([0x18, 0x18, 0x18, 255]));
    let gif = encode_gif(vec![Frame::new(lit.clone()), Frame::new(dark)]).unwrap();
    let r = run("status.online", "icons/status.online.gif", gif, Base::Dark);
    assert!(matches!(r, Err(SkinRule::Contrast { .. })), "{r:?}");
    let still = encode_gif(vec![Frame::new(lit.clone()), Frame::new(lit)]).unwrap();
    assert_eq!(
        run(
            "status.online",
            "icons/status.online.gif",
            still,
            Base::Dark
        ),
        Ok(())
    );
}
