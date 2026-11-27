//! Synthetic fixtures only: every theme, icon and sound is built here (no
//! third-party theme is committed; plan E7.7.3 *Decided (licence)*).

use rekindle_image::{encode_png, DynamicImage, RgbaImage};

use super::*;
use crate::import::Imported;
use crate::validate;

pub(crate) fn png(side: u32) -> Vec<u8> {
    let img = RgbaImage::from_pixel(side, side, rekindle_image::Rgba([40, 200, 90, 255]));
    encode_png(&DynamicImage::ImageRgba8(img)).unwrap()
}

/// A mono 8 kHz PCM16 WAV of `seconds`.
fn wav(seconds: u32) -> Vec<u8> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 8000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut out = std::io::Cursor::new(Vec::new());
    let mut w = hound::WavWriter::new(&mut out, spec).unwrap();
    for i in 0..8000 * seconds {
        w.write_sample(i16::try_from(i % 200).unwrap() * 50)
            .unwrap();
    }
    w.finalize().unwrap();
    out.into_inner()
}

const BLIST_XML: &str = r##"<?xml version="1.0"?>
<pidgin version="1" type="pidgin buddy list" name="Synthetic" author="test">
  <description>synthetic</description>
  <blist color="#1e1e1e"/>
  <groups>
    <expanded font="Sans Bold 10" color="#ffffff" background="#333333"/>
    <collapsed font="Sans 10" color="#cccccc" background="#222222"/>
  </groups>
  <buddys>
    <placement status_icon="0" name="1" emblem="2" protocol_icon="3" buddy_icon="4"/>
    <background color="#202020"/>
    <contact_text font="Sans 10" color="#e7e7e7"/>
    <online_text color="#53d769"/>
    <offline_text color="gray"/>
    <idle_text color="#808080"/>
    <status_text color="#969696"/>
  </buddys>
</pidgin>"##;

pub(crate) fn theme(icons: &str, sounds: &str) -> Tree {
    let mut t = Tree::new();
    t.insert("Synth/purple/blist/theme.xml".into(), BLIST_XML.into());
    t.insert(
        "Synth/purple/status-icon/theme.xml".into(),
        format!("<theme type=\"status-icon\" name=\"Synthetic\">{icons}</theme>").into_bytes(),
    );
    t.insert(
        "Synth/purple/sound/theme.xml".into(),
        format!("<theme type=\"sound\" name=\"Synthetic\">{sounds}</theme>").into_bytes(),
    );
    t.insert("Synth/purple/status-icon/16/available.png".into(), png(16));
    t.insert(
        "Synth/purple/status-icon/16/away.svg".into(),
        b"<svg/>".to_vec(),
    );
    t.insert("Synth/purple/sound/login.wav".into(), wav(1));
    t.insert("Synth/purple/sound/long.wav".into(), wav(6));
    t
}

pub(crate) const ICONS: &str = r#"<icon id="available" file="16/available.png"/>
    <icon id="extended-away" file="16/available.png"/>"#;
pub(crate) const SOUNDS: &str = r#"<event name="login" file="login.wav"/>
    <event name="send_im" file="login.wav"/>"#;

pub(crate) fn imported(outcome: Outcome) -> Imported {
    match outcome {
        Outcome::Passed(i) => i,
        Outcome::Draft(d) => {
            assert!(d.unsettled.is_none(), "unsettled: {:?}", d.unsettled);
            assert!(!d.failures.is_empty());
            d.suggested.expect("a draft with every failure suggested")
        }
    }
}

pub(crate) fn has(notice: &str, item: &str, why: &str) -> bool {
    notice.lines().any(|l| l.contains(item) && l.contains(why))
}

#[test]
fn a_minimal_theme_imports_and_validates() {
    let i = imported(import(&theme(ICONS, SOUNDS), "synth").unwrap());
    let c = validate::run(&i.canonical.bytes).unwrap();
    assert_eq!(c.address(), i.canonical.address());
    let tree = theme(ICONS, SOUNDS);
    let m = manifest(&tree, "synth", &read(&tree).unwrap(), "", SkinBase::Dark);
    assert_eq!(
        m.icons.get("status.online").map(String::as_str),
        Some("icons/status.online.png")
    );
    assert_eq!(
        m.sounds.get("sign-on").map(String::as_str),
        Some("sounds/sign-on.wav")
    );
    assert_eq!(m.icons.len(), 1);
    assert_eq!(m.sounds.len(), 1);
    let n = &i.notice;
    assert!(has(n, "blist blist color", "token bg.panel"), "{n}");
    assert!(has(n, "online_text color", "status.online-text"), "{n}");
    assert!(
        has(n, "offline_text color=\"gray\"", "not #rgb or #rrggbb"),
        "{n}"
    );
    assert!(has(n, "idle_text", "no idle status"), "{n}");
    assert!(has(n, "contact_text font=\"Sans 10\"", "Pango font"), "{n}");
    assert!(
        has(n, "icon extended-away", "no extended-away status"),
        "{n}"
    );
    assert!(has(n, "sound send_im", "sent message"), "{n}");
    // Deterministic: the same tree, the same bytes.
    let again = imported(import(&theme(ICONS, SOUNDS), "synth").unwrap());
    assert_eq!(again.canonical.bytes, i.canonical.bytes);
    assert_eq!(again.notice, i.notice);
}

/// An imported (non-built-in) theme validates with the model's dialog pairs
/// measured, and so does the same package with every dialog role removed,
/// on the roles `registry::resolve` serves it at load (`dialog::fill` and
/// the one dialog walk; ST1 rulings b, j). Fail-without: dropping the base
/// table's `holds_out` filter (`access/skin_model.rs`) leaves the import's
/// base walk unsettled on `UnmeasuredPair` (the dialog roles it holds out).
#[test]
fn a_minimal_theme_validates_with_its_dialog_pairs_measured() {
    use crate::registry::{dialog, resolve, Base, ENTRIES};
    let i = imported(import(&theme(ICONS, SOUNDS), "synth").unwrap());
    validate::run(&i.canonical.bytes).unwrap();
    let package = crate::package::read(&i.canonical.bytes).unwrap();
    let m = crate::manifest::parse(&package).unwrap();
    assert!(!m.variants.is_empty());
    for v in &m.variants {
        let base = Base::of(v.base, v.contrast);
        let file = package.get(&v.tokens).unwrap();
        let mut skin = crate::tokens::parse(&v.tokens, file).unwrap();
        skin.retain(|id, _| !dialog::is_role(id));
        let served = resolve(ENTRIES, base, &skin);
        let pairs = crate::access::measured(base, &served);
        assert!(
            pairs
                .iter()
                .any(|p| p.fg == "selection.text" && p.rule.starts_with("1.4.3 .")),
            "no model pair measured selection.text"
        );
        let failures = crate::access::failures(base, &served);
        assert!(failures.is_empty(), "{failures:?}");
    }
}

#[test]
fn every_emitted_icon_and_sound_key_is_a_registry_id() {
    use crate::registry::{IconSlot, SoundCue};
    use std::fmt::Write as _;
    let mut icons = String::new();
    for (id, _) in map::ICONS {
        let _ = write!(icons, r#"<icon id="{id}" file="16/available.png"/>"#);
    }
    let mut sounds = String::new();
    for (name, _) in map::SOUNDS {
        let _ = write!(sounds, r#"<event name="{name}" file="login.wav"/>"#);
    }
    let t = theme(&icons, &sounds);
    let i = imported(import(&t, "synth").unwrap());
    validate::run(&i.canonical.bytes).unwrap();
    let m = manifest(&t, "synth", &read(&t).unwrap(), "", SkinBase::Dark);
    assert_eq!(m.icons.len(), 5);
    assert_eq!(m.sounds.len(), 4);
    for (id, path) in &m.icons {
        assert!(IconSlot::of(id).is_some(), "icon {id}");
        assert_eq!(path, &format!("icons/{id}.png"));
    }
    for (id, path) in &m.sounds {
        assert!(SoundCue::of(id).is_some(), "sound {id}");
        assert_eq!(path, &format!("sounds/{id}.wav"));
    }
}

#[test]
fn an_entity_in_theme_xml_is_refused() {
    let mut t = theme(ICONS, SOUNDS);
    t.insert(
        "Synth/purple/sound/theme.xml".into(),
        b"<!DOCTYPE t [<!ENTITY x SYSTEM \"file:///etc/passwd\">]><theme type=\"sound\" name=\"&x;\"/>"
            .to_vec(),
    );
    assert!(matches!(import(&t, "synth"), Err(ImportError::Xml { .. })));
    let mut t = theme(ICONS, SOUNDS);
    t.insert(
        "Synth/purple/sound/theme.xml".into(),
        b"<theme type=\"sound\" name=\"s\"><event name=\"login\" file=\"login.wav\">&x;</event></theme>"
            .to_vec(),
    );
    assert!(matches!(import(&t, "synth"), Err(ImportError::Xml { .. })));
}

#[test]
fn an_unsupported_icon_format_is_dropped_and_named() {
    let icons = r#"<icon id="away" file="16/away.svg"/><icon id="busy" file="../busy.png"/>
        <icon id="offline" file="16/missing.png"/>"#;
    let i = imported(import(&theme(icons, SOUNDS), "synth").unwrap());
    assert!(read(&theme(icons, SOUNDS)).unwrap().icons.is_empty());
    let n = &i.notice;
    assert!(has(n, "icon away file=\"16/away.svg\"", "SVG"), "{n}");
    assert!(
        has(
            n,
            "icon busy file=\"../busy.png\"",
            "not a plain relative path"
        ),
        "{n}"
    );
    assert!(has(n, "icon offline", "not in the theme"), "{n}");
}

#[test]
fn a_sound_over_the_cue_limits_is_dropped_and_named() {
    let sounds = r#"<event name="logout" file="long.wav"/><event name="login" file="login.wav"/>"#;
    let i = imported(import(&theme(ICONS, sounds), "synth").unwrap());
    let m = read(&theme(ICONS, sounds)).unwrap();
    assert!(m.sounds.contains_key("sign-on"));
    assert!(!m.sounds.contains_key("sign-off"));
    assert!(
        has(
            &i.notice,
            "sound logout file=\"long.wav\"",
            "refused by the validator"
        ),
        "{}",
        i.notice
    );
}

#[test]
fn a_theme_without_its_loader_attributes_is_refused() {
    let mut t = theme(ICONS, SOUNDS);
    t.insert(
        "Synth/purple/blist/theme.xml".into(),
        b"<pidgin type=\"not a blist\" name=\"x\"/>".to_vec(),
    );
    assert!(matches!(import(&t, "synth"), Err(ImportError::Xml { .. })));
    assert!(matches!(
        import(&Tree::new(), "synth"),
        Err(ImportError::MissingFile { .. })
    ));
}

#[test]
fn the_theme_roots_author_is_credited() {
    let mut tree = theme(ICONS, SOUNDS);
    tree.insert(
        "Synth/purple/sound/theme.xml".into(),
        format!("<theme type=\"sound\" name=\"S\" author=\" Ann \">{SOUNDS}</theme>").into_bytes(),
    );
    tree.insert("README.txt".into(), b"Share alike.".to_vec());
    let m = manifest(&tree, "synth", &read(&tree).unwrap(), "", SkinBase::Dark);
    let credits = m.credits.expect("credits");
    assert_eq!(credits.authors, vec!["test".to_owned(), "Ann".to_owned()]);
    assert_eq!(credits.url, None);
    assert_eq!(
        credits.licence,
        rekindle_types::dto::skin::OriginalLicence::Text("Share alike.".into())
    );
}
