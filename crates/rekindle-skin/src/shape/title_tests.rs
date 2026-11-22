//! The title strip's refusals and rules (plan SK1-S1). Each refusal test
//! goes through [`crate::shape::check`], the helper's entry.

use std::collections::BTreeMap;

use rekindle_types::dto::skin::{RegionState, SkinManifest, SkinWindow, TileAxis};
use rekindle_types::dto::skin_title::{TitleLayout, TitlePart, TitleRole, TitleSlices, TitleText};

use super::*;
use crate::{builtin, images, manifest, package};

fn pack() -> (SkinManifest, BTreeMap<String, Vec<u8>>) {
    let (_, b) = builtin::find("builtin:symbiosis").unwrap();
    let p = package::read(b.bytes).unwrap();
    let m = manifest::parse(&p).unwrap();
    let files = images::validate(&p, &m).unwrap();
    (m, files)
}

fn button(
    m: &SkinManifest,
    files: &BTreeMap<String, Vec<u8>>,
    role: TitleRole,
    right: u16,
) -> TitlePart {
    let region = &m.regions[&format!("titlebar.btn.{}", role.key())];
    let (w, h) = dimensions(&region.image, files).unwrap();
    TitlePart {
        role,
        x: TileAxis::End { offset: right },
        y: 8,
        width: u16::try_from(w).unwrap(),
        height: u16::try_from(h).unwrap(),
        image: Some(region.image.clone()),
        states: region
            .states
            .clone()
            .into_iter()
            .filter(|(s, _)| *s != RegionState::Disabled)
            .collect(),
        text: None,
        slices: None,
    }
}

fn name() -> TitlePart {
    TitlePart {
        role: TitleRole::Name,
        x: TileAxis::Start { offset: 20 },
        y: 10,
        width: 0,
        height: 16,
        image: None,
        states: BTreeMap::new(),
        text: Some(TitleText {
            font: "Arial".to_owned(),
            size: 12,
            bold: true,
            colour: Some("#ffcc00".to_owned()),
            inset: [0, 0],
        }),
        slices: None,
    }
}

/// Symbiosis with a valid strip: minimise and close, the name field.
fn laid() -> (SkinManifest, BTreeMap<String, Vec<u8>>) {
    let (mut m, files) = pack();
    let parts = vec![
        button(&m, &files, TitleRole::Minimize, 47),
        button(&m, &files, TitleRole::Close, 23),
        name(),
    ];
    m.windows.get_mut(&SkinWindow::Main).unwrap().title = Some(TitleLayout { bottom: 51, parts });
    (m, files)
}

fn refused(edit: impl FnOnce(&mut TitleLayout, &BTreeMap<String, Vec<u8>>)) -> String {
    let (mut m, files) = laid();
    let t = m
        .windows
        .get_mut(&SkinWindow::Main)
        .unwrap()
        .title
        .as_mut()
        .unwrap();
    edit(t, &files);
    crate::shape::check(&m, &files).unwrap_err().entry
}

#[test]
fn a_valid_strip_passes_and_lays_the_pack_images_without_glyphs() {
    let (m, files) = laid();
    crate::shape::check(&m, &files).unwrap();
    let css = crate::shape::frame_css(&m, &files, "skin://x").unwrap();
    assert!(css.contains("  .app-frame[data-skin-window] .xfire-titlebar-btn-close {\n    display: block; position: absolute; right: 23px; top: 8px;"), "{css}");
    assert!(css.contains(
        ".app-frame[data-skin-window] .xfire-titlebar-btn-close > .xfire-titlebar-glyph {\n    display: none;"
    ));
    assert!(css.contains(
        ".app-frame[data-skin-window] .xfire-titlebar-btn-close:hover {\n    background-image: url(\"skin://x/"
    ));
    assert!(css.contains(
        ".app-frame[data-skin-window] .xfire-titlebar-btn-close:active {\n    background-image: url(\"skin://x/"
    ));
    assert!(css.contains("width: max(100%, 24px);"));
    assert!(css.contains("font-family: \"Arial\"; font-size: 12px; font-weight: bold;"));
    // The strip row reaches the strip's bottom (51) from the content top; under
    // SK2 the content is the views box (top 237), so the strip lies wholly
    // above it and the row reserves nothing.
    assert!(css.contains(".app-frame[data-skin-window] .xfire-titlebar {\n    height: 0px;"));
}

#[test]
fn a_strip_without_a_close_button_is_refused() {
    let e = refused(|t, _| t.parts.retain(|p| p.role != TitleRole::Close));
    assert_eq!(e, "windows.main.title.parts");
}

#[test]
fn two_parts_of_one_role_are_refused() {
    let e = refused(|t, _| t.parts.push(name()));
    assert_eq!(e, "windows.main.title.parts");
}

#[test]
fn a_part_below_the_strip_is_refused() {
    let e = refused(|t, _| t.bottom = 20);
    assert_eq!(e, "windows.main.title.parts.0.y");
}

#[test]
fn a_strip_past_the_limit_is_refused() {
    let e = refused(|t, _| t.bottom = MAX_BOTTOM + 1);
    assert_eq!(e, "windows.main.title.bottom");
}

#[test]
fn a_button_not_at_its_image_size_is_refused() {
    let e = refused(|t, _| t.parts[1].width += 1);
    assert_eq!(e, "windows.main.title.parts.1.states");
    let e = refused(|t, _| {
        t.parts[1].width += 1;
        t.parts[1].states.clear();
    });
    assert_eq!(e, "windows.main.title.parts.1.image");
}

#[test]
fn a_button_without_an_image_or_with_a_wrong_state_is_refused() {
    let e = refused(|t, _| t.parts[1].image = None);
    assert_eq!(e, "windows.main.title.parts.1.image");
    let e = refused(|t, _| {
        t.parts[1]
            .states
            .insert(RegionState::Hover, "assets/titlebar.png".to_owned());
    });
    assert_eq!(e, "windows.main.title.parts.1.states");
    let e = refused(|t, _| {
        let img = t.parts[1].image.clone().unwrap();
        t.parts[1].states.insert(RegionState::Disabled, img);
    });
    assert_eq!(e, "windows.main.title.parts.1.states");
}

#[test]
fn a_centred_or_stretched_button_is_refused() {
    assert_eq!(
        refused(|t, _| t.parts[1].x = TileAxis::Centre),
        "windows.main.title.parts.1.x"
    );
    assert_eq!(
        refused(|t, _| t.parts[1].x = TileAxis::Repeat { start: 0, end: 0 }),
        "windows.main.title.parts.1.x"
    );
}

#[test]
fn a_logo_not_in_the_package_or_not_its_size_is_refused() {
    let logo = |image: &str, width| TitlePart {
        role: TitleRole::Logo,
        x: TileAxis::Start { offset: 9 },
        y: 10,
        width,
        height: 13,
        image: Some(image.to_owned()),
        states: BTreeMap::new(),
        text: None,
        slices: None,
    };
    let e = refused(|t, _| t.parts.push(logo("assets/none.png", 13)));
    assert_eq!(e, "windows.main.title.parts.3.image");
    let e = refused(|t, _| t.parts.push(logo("assets/frame/main/top-left.png", 14)));
    assert_eq!(e, "windows.main.title.parts.3.image");
}

#[test]
fn a_name_face_that_could_escape_css_or_is_out_of_range_is_refused() {
    let face = |font: &'static str, size| {
        move |t: &mut TitleLayout, _: &BTreeMap<String, Vec<u8>>| {
            t.parts[2].text = Some(TitleText {
                font: font.to_owned(),
                size,
                bold: false,
                colour: None,
                inset: [0, 0],
            });
        }
    };
    assert_eq!(
        refused(face("Arial\"; } * { x", 12)),
        "windows.main.title.parts.2.text"
    );
    assert_eq!(refused(face("Arial", 7)), "windows.main.title.parts.2.text");
    assert_eq!(
        refused(face("Arial", 33)),
        "windows.main.title.parts.2.text"
    );
    assert_eq!(
        refused(|t, _| t.parts[2].text = None),
        "windows.main.title.parts.2.text"
    );
}

#[test]
fn a_name_colour_that_is_not_hex_rgb_is_refused() {
    let colour = |c: &'static str| {
        move |t: &mut TitleLayout, _: &BTreeMap<String, Vec<u8>>| {
            t.parts[2].text.as_mut().unwrap().colour = Some(c.to_owned());
        }
    };
    assert_eq!(
        refused(colour("red; } * { x")),
        "windows.main.title.parts.2.text"
    );
    assert_eq!(refused(colour("#ffcc0")), "windows.main.title.parts.2.text");
}

#[test]
fn name_slices_not_at_the_part_height_are_refused() {
    let e = refused(|t, _| {
        t.parts[2].slices = Some(TitleSlices {
            left: "assets/frame/main/top-left.png".to_owned(),
            middle: "assets/frame/main/top-left.png".to_owned(),
            right: "assets/none.png".to_owned(),
        });
    });
    assert_eq!(e, "windows.main.title.parts.2.slices");
}

#[test]
fn the_name_field_draws_its_pack_colour_and_slices() {
    let (mut m, files) = laid();
    let (w, h) = dimensions("assets/frame/main/top-left.png", &files).unwrap();
    let title = m
        .windows
        .get_mut(&SkinWindow::Main)
        .unwrap()
        .title
        .as_mut()
        .unwrap();
    let p = &mut title.parts[2];
    p.height = u16::try_from(h).unwrap();
    let art = "assets/frame/main/top-left.png".to_owned();
    p.slices = Some(TitleSlices {
        left: art.clone(),
        middle: art.clone(),
        right: art,
    });
    p.text.as_mut().unwrap().inset = [u16::try_from(w).unwrap() + 3, 0];
    title.bottom = title.bottom.max(10 + u16::try_from(h).unwrap());
    crate::shape::title::check(&m, &files).unwrap();
    let css = crate::shape::title::title_css(
        ".app-frame",
        m.windows[&SkinWindow::Main].title.as_ref().unwrap(),
        0,
        &files,
        "skin://x/",
    )
    .unwrap();
    assert!(css.contains("color: #ffcc00;"), "{css}");
    assert!(
        css.contains(&format!("border-width: 0 {w}px 0 {w}px;")),
        "{css}"
    );
    assert!(css.contains("padding: 0 0px 0 3px;"), "{css}");
    assert!(css.contains("repeat-x padding-box padding-box"), "{css}");
}
