//! Region slots beyond the four: per-state images (plan E7.7.1 "Regions")
//! and the region CSS.

use std::collections::BTreeMap;

use rekindle_image::{encode_png, DynamicImage, Rgba, RgbaImage};
use rekindle_skin::regions::css::{app_css, skin_css};
use rekindle_skin::registry::{resolve, Base, ENTRIES};
use rekindle_skin::{images, manifest, package, regions};
use rekindle_types::dto::skin::{SkinError, SkinManifest, SkinRule};
use serde_json::{json, Value};

use crate::common::{self, Entry};

fn png(w: u32, h: u32) -> Vec<u8> {
    let image = RgbaImage::from_pixel(w, h, Rgba([40, 60, 90, 255]));
    encode_png(&DynamicImage::ImageRgba8(image)).unwrap()
}

/// A `region` with a normal image and `states` (state → image size).
fn manifest_with(region: &str, fit: &Value, states: &[(&str, (u32, u32))]) -> Value {
    let mut m = common::manifest();
    let st: serde_json::Map<String, Value> = states
        .iter()
        .map(|(s, _)| ((*s).to_owned(), json!(format!("assets/{s}.png"))))
        .collect();
    m["regions"] = json!({ region: { "image": "assets/normal.png", "fit": fit, "states": st } });
    m
}

type Checked = (SkinManifest, BTreeMap<String, Vec<u8>>);

fn check(
    m: &Value,
    normal: (u32, u32),
    states: &[(&str, (u32, u32))],
) -> Result<Checked, SkinError> {
    let mut entries = vec![
        Entry::file("manifest.json", m.to_string().as_bytes()),
        Entry::file("dark.json", b"{}"),
        Entry::file("assets/normal.png", &png(normal.0, normal.1)),
    ];
    let stored: Vec<(String, Vec<u8>)> = states
        .iter()
        .map(|(s, (w, h))| (format!("assets/{s}.png"), png(*w, *h)))
        .collect();
    for (name, bytes) in &stored {
        entries.push(Entry::file(name, bytes));
    }
    let bytes = common::zip(&entries);
    let package = package::read(&bytes)?;
    let manifest = manifest::parse(&package)?;
    let canonical = images::validate(&package, &manifest)?;
    let resolved = resolve(ENTRIES, Base::Dark, &BTreeMap::new());
    regions::validate(&manifest, &canonical, &resolved)?;
    Ok((manifest, canonical))
}

fn nine() -> Value {
    json!({ "type": "nineSlice", "data": { "top": 2, "right": 3, "bottom": 2, "left": 3 } })
}

#[test]
fn a_button_takes_hover_down_and_disabled_images() {
    let states = [
        ("hover", (18, 16)),
        ("down", (18, 16)),
        ("disabled", (18, 16)),
    ];
    let m = manifest_with("titlebar.btn.close", &nine(), &states);
    check(&m, (18, 16), &states).unwrap();
}

#[test]
fn a_state_image_of_another_size_is_refused() {
    let states = [("hover", (18, 16)), ("down", (19, 16))];
    let m = manifest_with("titlebar.btn.close", &nine(), &states);
    let err = check(&m, (18, 16), &states).expect_err("down is wider");
    assert_eq!(err.entry, "assets/down.png");
    assert_eq!(
        err.rule,
        SkinRule::RegionStateSize {
            region: "titlebar.btn.close".to_owned(),
            state: "down".to_owned(),
            width: 19,
            height: 16,
            normal_width: 18,
            normal_height: 16,
        }
    );
}

#[test]
fn a_state_the_slot_lacks_is_refused() {
    let states = [("hover", (8, 8))];
    let m = manifest_with("statusbar", &json!({ "type": "stretch" }), &states);
    let mut m = m;
    m["regions"]["statusbar"]["textBacking"] = json!("bg.status");
    let err = check(&m, (8, 8), &states).expect_err("the status bar has no hover");
    assert_eq!(
        err.rule,
        SkinRule::UnsupportedRegionState {
            region: "statusbar".to_owned(),
            state: "hover".to_owned(),
        }
    );
    let talking = [("talking", (8, 8))];
    let m = manifest_with("scrollbar.thumb", &json!({ "type": "stretch" }), &talking);
    assert!(matches!(
        check(&m, (8, 8), &talking).unwrap_err().rule,
        SkinRule::UnsupportedRegionState { .. }
    ));
}

#[test]
fn the_skin_css_addresses_images_by_hash_under_the_base() {
    let states = [("hover", (18, 16))];
    let m = manifest_with("titlebar.btn.close", &nine(), &states);
    let (manifest, canonical) = check(&m, (18, 16), &states).unwrap();
    let css = skin_css(&manifest, &canonical, "skin://localhost/abc").unwrap();
    let normal = blake3::hash(&canonical["assets/normal.png"]).to_hex();
    let hover = blake3::hash(&canonical["assets/hover.png"]).to_hex();
    assert_eq!(
        css,
        format!(
            "@layer skin {{\n  :root {{\n    --skin-region-titlebar-btn-close: url(\"skin://localhost/abc/{normal}.png\") 2 3 2 3 fill / 2px 3px 2px 3px / 0 stretch;\n    --skin-region-titlebar-btn-close--hover: url(\"skin://localhost/abc/{hover}.png\") 2 3 2 3 fill / 2px 3px 2px 3px / 0 stretch;\n  }}\n}}\n"
        )
    );
    assert!(!css.contains("assets/"), "no package name reaches CSS");
    assert!(skin_css(&manifest, &canonical, "skin://x\") }").is_none());
}

fn map(entries: &[(&str, &[&str])]) -> BTreeMap<String, Vec<String>> {
    entries
        .iter()
        .map(|(k, v)| ((*k).to_owned(), v.iter().map(|s| (*s).to_owned()).collect()))
        .collect()
}

/// Every painted slot on one test element each.
fn full_map() -> BTreeMap<String, Vec<String>> {
    map(&[
        ("titlebar.btn.minimize", &[".min"]),
        ("titlebar.btn.maximize", &[".max"]),
        ("titlebar.btn.close", &[".close"]),
        ("scrollbar.track", &["::-webkit-scrollbar-track:vertical"]),
        ("scrollbar.thumb", &["::-webkit-scrollbar-thumb:vertical"]),
        (
            "scrollbar.track-horizontal",
            &["::-webkit-scrollbar-track:horizontal"],
        ),
        (
            "scrollbar.thumb-horizontal",
            &["::-webkit-scrollbar-thumb:horizontal"],
        ),
        (
            "scrollbar.button-up",
            &["::-webkit-scrollbar-button:vertical:decrement"],
        ),
        (
            "scrollbar.button-down",
            &["::-webkit-scrollbar-button:vertical:increment"],
        ),
        (
            "scrollbar.button-left",
            &["::-webkit-scrollbar-button:horizontal:decrement"],
        ),
        (
            "scrollbar.button-right",
            &["::-webkit-scrollbar-button:horizontal:increment"],
        ),
        ("scrollbar.corner", &["::-webkit-scrollbar-corner"]),
        ("button.secondary", &[".btn2"]),
    ])
}

#[test]
fn the_app_css_maps_states_to_selectors() {
    let full = full_map();
    let css = app_css("h", &full).unwrap();
    assert!(css.starts_with("/* h\n"));
    assert!(css.contains("@layer components {"));
    assert!(css.contains("  .close {\n    border-image: var(--skin-region-titlebar-btn-close);\n    image-rendering: pixelated;\n  }\n"));
    assert!(css.contains("  .close:hover {\n    border-image: var(--skin-region-titlebar-btn-close--hover, var(--skin-region-titlebar-btn-close));\n  }\n"));
    assert!(css.contains(
        "  .close:active {\n    border-image: var(--skin-region-titlebar-btn-close--down,"
    ));
    assert!(css.contains("  .close:disabled, .close[aria-disabled=\"true\"] {\n"));
    assert!(css.contains("  :where(:root[data-window-inactive]) .close {\n"));
    assert!(css.contains("  .close:focus-visible {\n"));
    // A pseudo-element takes WebKit's pseudo-classes, never an attribute.
    assert!(css.contains("  ::-webkit-scrollbar-thumb:vertical:disabled {\n"));
    assert!(css.contains("  ::-webkit-scrollbar-thumb:vertical:window-inactive {\n"));
    assert!(!css.contains("scrollbar-thumb:vertical[aria-disabled"));
    // Disabled comes after down, down after hover: the later state wins.
    let at = |s: &str| css.find(s).unwrap();
    assert!(
        at(".close:hover") < at(".close:active") && at(".close:active") < at(".close:disabled")
    );

    let mut missing = full.clone();
    missing.remove("scrollbar.corner");
    assert!(app_css("h", &missing)
        .unwrap_err()
        .contains("scrollbar.corner"));
    let mut unknown = full.clone();
    unknown.insert("titlebar".to_owned(), vec![".xfire-titlebar".to_owned()]);
    assert!(app_css("h", &unknown)
        .unwrap_err()
        .contains("not a painted region slot"));
}

#[test]
fn the_app_css_resets_every_region_rule_under_forced_colors() {
    let css = app_css("h", &full_map()).unwrap();
    let media = css.find("  @media (forced-colors: active) {\n").unwrap();
    // After every painted rule, so it wins at each rule's own specificity.
    assert!(css.rfind("var(--skin-region-").unwrap() < media);
    assert!(css.ends_with("    }\n  }\n}\n"));
    let forced = &css[media..];
    for list in [
        ".close",
        ".close:hover",
        ".close:active",
        ".close:focus-visible",
        ".close:disabled, .close[aria-disabled=\"true\"]",
        ":where(:root[data-window-inactive]) .close",
        "::-webkit-scrollbar-thumb:vertical:window-inactive",
        "::-webkit-scrollbar-corner",
    ] {
        assert!(
            forced.contains(&format!(
                "    {list} {{\n      border-image: none;\n    }}\n"
            )),
            "{list}"
        );
    }
    let painted = css.matches("border-image: var(").count();
    assert_eq!(forced.matches("border-image: none;").count(), painted);
}

#[test]
fn the_app_css_sizes_scrollbar_buttons_from_the_skin() {
    let css = app_css("h", &full_map()).unwrap();
    assert!(css.contains("  ::-webkit-scrollbar-button:vertical:decrement {\n    border-image: var(--skin-region-scrollbar-button-up);\n    image-rendering: pixelated;\n    height: var(--skin-region-scrollbar-button-up-length, 0);\n  }\n"));
    assert!(css.contains("    width: var(--skin-region-scrollbar-button-right-length, 0);\n"));
    assert!(css.contains("    height: var(--skin-region-scrollbar-button-down-length, 0);\n"));
    assert!(css.contains("    width: var(--skin-region-scrollbar-button-left-length, 0);\n"));
    assert_eq!(css.matches("-length, 0);").count(), 4, "buttons only");
}

#[test]
fn the_skin_css_gives_a_scrollbar_button_its_image_length() {
    let up = manifest_with("scrollbar.button-up", &json!({ "type": "stretch" }), &[]);
    let (manifest, canonical) = check(&up, (14, 12), &[]).unwrap();
    let css = skin_css(&manifest, &canonical, "skin://localhost/abc").unwrap();
    assert!(css.contains("    --skin-region-scrollbar-button-up-length: 12px;\n"));
    let left = manifest_with("scrollbar.button-left", &json!({ "type": "stretch" }), &[]);
    let (manifest, canonical) = check(&left, (14, 12), &[]).unwrap();
    let css = skin_css(&manifest, &canonical, "skin://localhost/abc").unwrap();
    assert!(css.contains("    --skin-region-scrollbar-button-left-length: 14px;\n"));
}

#[test]
fn the_app_css_backs_a_text_slot_between_image_and_label() {
    let css = app_css("h", &full_map()).unwrap();
    // The element is the stacking context; its `::before` paints the backing
    // at z-index -1: after the border-image, under the label (CSS 2.1 App. E).
    assert!(css.contains("  .btn2 {\n    border-image: var(--skin-region-button-secondary);\n    image-rendering: pixelated;\n    position: relative;\n    isolation: isolate;\n  }\n"));
    // Its backing is the image's centre (the padding box inset by the
    // slice widths, 0 with no skin) and the label's content box.
    let fill = "linear-gradient(var(--skin-region-button-secondary-backing), var(--skin-region-button-secondary-backing))";
    let [t, r, b, l] = ["top", "right", "bottom", "left"]
        .map(|side| format!("var(--skin-region-button-secondary-slice-{side}, 0px)"));
    let before = format!("  .btn2::before {{\n    content: \"\";\n    position: absolute;\n    inset: 0;\n    z-index: -1;\n    padding: inherit;\n    border-radius: inherit;\n    background:\n      {fill} {l} {t} / calc(100% - {l} - {r}) calc(100% - {t} - {b}) no-repeat padding-box,\n      {fill} no-repeat content-box;\n    pointer-events: none;\n  }}\n");
    assert!(css.contains(&before), "{css}");
    assert_eq!(
        css.matches("isolation: isolate;").count(),
        1,
        "text slots only"
    );
    // Under forced colors the backing is not generated.
    let forced = &css[css.find("  @media (forced-colors: active) {\n").unwrap()..];
    assert!(forced.contains("    .btn2::before {\n      content: none;\n    }\n"));
    // A text slot on a pseudo-element cannot take a `::before`.
    let mut pseudo = full_map();
    pseudo.insert(
        "button.secondary".to_owned(),
        vec!["::-webkit-scrollbar".to_owned()],
    );
    assert!(app_css("h", &pseudo)
        .unwrap_err()
        .contains("pseudo-element"));
}

#[test]
fn the_skin_css_writes_a_text_slot_backing_token() {
    let mut m = manifest_with("button.secondary", &nine(), &[]);
    m["regions"]["button.secondary"]["textBacking"] = json!("bg.panel");
    let (manifest, canonical) = check(&m, (18, 16), &[]).unwrap();
    let css = skin_css(&manifest, &canonical, "skin://localhost/abc").unwrap();
    assert!(
        css.contains("    --skin-region-button-secondary-backing: var(--color-xfire-bg-panel);\n")
    );
}

#[test]
fn the_skin_css_writes_a_text_slot_slice_widths() {
    // `nine()` is 2/3/2/3: the backing's centre layer starts there.
    let mut m = manifest_with("button.secondary", &nine(), &[]);
    m["regions"]["button.secondary"]["textBacking"] = json!("bg.panel");
    let (manifest, canonical) = check(&m, (18, 16), &[]).unwrap();
    let css = skin_css(&manifest, &canonical, "skin://localhost/abc").unwrap();
    for (side, px) in [("top", 2), ("right", 3), ("bottom", 2), ("left", 3)] {
        let decl = format!("    --skin-region-button-secondary-slice-{side}: {px}px;\n");
        assert!(css.contains(&decl), "{decl}");
    }
    // A stretched image has no slices: the backing is the whole box.
    let mut m = manifest_with("button.secondary", &json!({ "type": "stretch" }), &[]);
    m["regions"]["button.secondary"]["textBacking"] = json!("bg.panel");
    let (manifest, canonical) = check(&m, (18, 16), &[]).unwrap();
    let css = skin_css(&manifest, &canonical, "skin://localhost/abc").unwrap();
    assert!(css.contains("    --skin-region-button-secondary-slice-left: 0px;\n"));
    // Only text slots carry slice widths.
    let m = manifest_with("scrollbar.thumb", &nine(), &[]);
    let (manifest, canonical) = check(&m, (18, 16), &[]).unwrap();
    let css = skin_css(&manifest, &canonical, "skin://localhost/abc").unwrap();
    assert!(!css.contains("-slice-"));
}
