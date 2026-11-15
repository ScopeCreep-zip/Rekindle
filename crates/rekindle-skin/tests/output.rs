//! Validator step 7: the canonical package and the `tokens.css` body (plan
//! E7.7.1 Verify).

use std::collections::BTreeMap;

use rekindle_image::{encode_png, DynamicImage, Rgba, RgbaImage};
use rekindle_skin::output::{self, Canonical};
use rekindle_skin::registry::{font, resolve, Base, TokenType, ENTRIES};
use rekindle_skin::tokens::emit::in_alphabet;
use rekindle_skin::{images, manifest, package, sounds, tokens};
use rekindle_types::dto::skin::{SkinManifest, TokenValue};

use crate::common::{self, Entry};

const DARK: &[u8] = br##"{"bg":{"panel":{"$type":"color","$value":"#1B2B3CFF"}},
  "size":{"chrome-height":{"$type":"dimension","$value":{"value":30,"unit":"px"}}},
  "tint":{"fill":{"$type":"number","$value":0.55}}}"##;

fn png(shade: u8) -> Vec<u8> {
    let image = RgbaImage::from_pixel(3, 2, Rgba([shade, 40, 90, 255]));
    encode_png(&DynamicImage::ImageRgba8(image)).unwrap()
}

/// Steps 1–5 and 7 over `entries`.
fn canonical(entries: &[Entry]) -> (SkinManifest, BTreeMap<String, TokenValue>, Canonical) {
    let bytes = common::zip(entries);
    let package = package::read(&bytes).unwrap();
    let manifest = manifest::parse(&package).unwrap();
    let mut parsed = BTreeMap::new();
    for v in &manifest.variants {
        let file = tokens::parse(&v.tokens, package.get(&v.tokens).unwrap()).unwrap();
        parsed.insert(v.tokens.clone(), file);
    }
    let images = images::validate(&package, &manifest).unwrap();
    let sounds = sounds::validate(&package, &manifest).unwrap();
    let out = output::package(&manifest, &parsed, &images, &sounds).unwrap();
    let dark = parsed.remove("dark.json").unwrap();
    (manifest, dark, out)
}

fn entries() -> Vec<Entry> {
    vec![
        Entry::file("manifest.json", common::manifest().to_string().as_bytes()),
        Entry::file("dark.json", DARK),
        Entry::file("assets/a.png", &png(10)),
        Entry::file("assets/b.png", &png(200)),
        Entry::file("icons/c.png", &png(90)),
    ]
}

#[test]
fn the_address_does_not_depend_on_entry_order() {
    let forward = entries();
    let mut backward = entries();
    backward.reverse();
    // The inputs differ as archives: hashing what came in would differ.
    assert_ne!(common::zip(&forward), common::zip(&backward));
    let (_, _, a) = canonical(&forward);
    let (_, _, b) = canonical(&backward);
    assert_eq!(a.hash, b.hash);
    assert_eq!(a.bytes, b.bytes);
    assert_eq!(a.hash, *blake3::hash(&a.bytes).as_bytes());
    assert_eq!(a.address().len(), 64);
}

#[test]
fn the_canonical_package_reads_back_as_the_same_skin() {
    let (manifest, dark, out) = canonical(&entries());
    let package = package::read(&out.bytes).unwrap();
    assert_eq!(manifest::parse(&package).unwrap(), manifest);
    let back = tokens::parse("dark.json", package.get("dark.json").unwrap()).unwrap();
    assert_eq!(back, dark);
    let names: Vec<_> = package.files().map(|(n, _)| n.to_owned()).collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted);
}

#[test]
fn the_css_body_holds_only_registry_names_and_re_emitted_values() {
    let (_, dark, _) = canonical(&entries());
    let resolved = resolve(ENTRIES, Base::Dark, &dark);
    let css = output::tokens_css(&resolved).unwrap();
    let body = css
        .strip_prefix("@layer skin {\n  :root {\n")
        .and_then(|b| b.strip_suffix("  }\n}\n"))
        .expect("@layer skin { :root { … } }");
    let lines: Vec<_> = body.lines().collect();
    assert_eq!(lines.len(), resolved.len());
    for ((e, v), line) in resolved.iter().zip(lines) {
        let value = line
            .strip_prefix(&format!("    {}: ", e.css))
            .and_then(|l| l.strip_suffix(';'))
            .unwrap_or_else(|| panic!("{line}"));
        if let TokenValue::FontFamily(name) = v {
            assert!(matches!(e.ty, TokenType::FontFamily), "{line}");
            assert_eq!(font(name).map(|f| f.stack), Some(value), "{line}");
        } else {
            assert!(in_alphabet(value), "{line}");
        }
    }
    assert!(css.contains("--color-xfire-bg-panel: #1b2b3c;"), "{css}");
}
