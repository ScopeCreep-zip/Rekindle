//! Validator step 2: `manifest.json` and its text rules (plan E7.7.1).
//!
//! The format major is read first, from the `format` key alone, so a
//! manifest of an unknown major is refused as such and never partly read.
//! The manifest is then parsed with `deny_unknown_fields`, and each field
//! checked: `id`, `name` (≤ 64 scalars, NFC, no control or bidi
//! characters), `description` (≤ 280), `license` (an SPDX expression or the
//! full text), `source`, the human `credits` (plan E7.7.3d: each name as
//! `name`, ≤ 16 per list, an `http(s)` URL ≤ 512, the original licence text
//! as `license`'s), `author` with `signature`, and the variants (one or
//! more, one per base and contrast, each naming a root token file the
//! package holds). The icons and sounds keys are checked here against the
//! registry's icon slots and sound cues; the regions map against the
//! registry, and the images and sounds, by the later validator steps.

pub(crate) mod text;

use std::collections::HashSet;

use rekindle_types::dto::skin::{
    OriginalLicence, SkinBase, SkinContrast, SkinCredits, SkinError, SkinLicense, SkinManifest,
    SkinRule, SKIN_FORMAT_MAJOR, SKIN_FORMAT_PREFIX,
};
use serde::Deserialize;

use crate::limits::MAX_MANIFEST_BYTES;
use crate::package::{is_root_json, Package, MANIFEST};
use crate::registry::{IconSlot, SoundCue};
use crate::skin_tree::dim::Made;
use text::TextRule;

const NAME: TextRule = TextRule {
    max_scalars: 64,
    may_be_empty: false,
    line_feeds: false,
};
const DESCRIPTION: TextRule = TextRule {
    max_scalars: 280,
    may_be_empty: true,
    line_feeds: true,
};
/// A licence's full text: bounded by the manifest's own size.
const LICENSE_TEXT: TextRule = TextRule {
    max_scalars: 64 * 1024,
    may_be_empty: false,
    line_feeds: true,
};
/// The most names in `credits.authors` or `credits.contributors`.
pub const MAX_CREDIT_NAMES: usize = 16;
/// The longest `credits.url`, in bytes (ASCII only).
pub const MAX_CREDIT_URL: usize = 512;
const ID_MAX: usize = 64;
const ED25519_KEY_HEX: usize = 64;
const ED25519_SIGNATURE_HEX: usize = 128;
const BLAKE3_HEX: usize = 64;

/// Only the format key, every other key ignored, to read the major first.
#[derive(Deserialize)]
struct FormatProbe {
    format: String,
}

fn refuse(rule: SkinRule) -> SkinError {
    SkinError::new(MANIFEST, rule)
}

fn bad_value(field: &str, expected: &str) -> SkinError {
    refuse(SkinRule::BadValue {
        field: field.to_owned(),
        expected: expected.to_owned(),
    })
}

fn syntax(e: &serde_json::Error) -> SkinError {
    refuse(SkinRule::ManifestSyntax {
        detail: e.to_string(),
    })
}

/// Parses and checks the package's manifest.
///
/// # Errors
/// The first rule the manifest breaks; the entry is `manifest.json`, or the
/// token file a variant names when the package lacks it.
pub fn parse(package: &Package<'_>) -> Result<SkinManifest, SkinError> {
    let bytes = package
        .get(MANIFEST)
        .ok_or_else(|| refuse(SkinRule::MissingManifest))?;
    let size = bytes.len() as u64;
    if size > MAX_MANIFEST_BYTES {
        return Err(refuse(SkinRule::ManifestTooLarge {
            size,
            max: MAX_MANIFEST_BYTES,
        }));
    }
    let probe: FormatProbe = serde_json::from_slice(bytes).map_err(|e| syntax(&e))?;
    check_format(&probe.format)?;
    let manifest: SkinManifest = serde_json::from_slice(bytes).map_err(|e| syntax(&e))?;
    check_fields(&manifest)?;
    check_variants(package, &manifest)?;
    check_keys(&manifest)?;
    Ok(manifest)
}

/// The manifest as the Host serves and validates it: [`parse`]'s, the
/// author's own, plus the Host's placements (rule N4, plan SK2-P1: the
/// Communities view and its control in the main window's tree). The
/// placements are never written into a package: the canonical bytes, a
/// draft export and the login copy stay the author's (design `sk2a` §3:
/// "the Host places"), and every read of a package places again.
///
/// # Errors
/// As [`parse`].
pub fn served(package: &Package<'_>) -> Result<SkinManifest, SkinError> {
    served_made(package).map(|(manifest, _)| manifest)
}

/// [`served`] and the images its placements made ([`place`]).
///
/// # Errors
/// As [`parse`].
pub fn served_made(package: &Package<'_>) -> Result<(SkinManifest, Made), SkinError> {
    let mut manifest = parse(package)?;
    let made = place(&mut manifest, package)?;
    crate::skin_tree::brand::texts(&mut manifest);
    Ok((manifest, made))
}

/// The Host's placements in `manifest` (see [`served`]): each button row's
/// slots unpaged by the views' group ([`crate::skin_tree::row_pages`]), then the cover of
/// each glyph label from the package's art (plan SK3: the icon the
/// Communities clone copied, [`crate::skin_tree::cover`]), then every
/// window's menu-title labels (plan SK4b, [`crate::skin_tree::titles`]) and
/// text-field insets (plan SK5, [`crate::skin_tree::inset`]) and the target
/// boxes of small interactive nodes (plan SK7, [`crate::skin_tree::target`])
/// and who dims each one (plan CR1, [`crate::skin_tree::disabled`]),
/// then the Community window composed from the pack's own windows (plan
/// SK17-6, [`crate::skin_tree::compose`]). Returns the images it made: the
/// Disabled copies the serve draws ([`crate::skin_tree::dim`]), served
/// beside the package's own and never written into it.
///
/// # Errors
/// A glyph label's or menu title's picture, or a disabled node's Normal or
/// Disabled piece, missing from `package` or not decoding.
pub fn place(manifest: &mut SkinManifest, package: &Package<'_>) -> Result<Made, SkinError> {
    // Plan SK15b: content texts sized to the served words and every chain
    // resolved first, so each pass after lays numeric spans.
    for layout in manifest.windows.values_mut() {
        crate::skin_tree::content::serve(layout);
        crate::skin_tree::row_pages::rows(layout);
    }
    if let Some(main) = manifest
        .windows
        .get_mut(&rekindle_types::dto::skin::SkinWindow::Main)
    {
        crate::skin_tree::place::communities(main);
        crate::skin_tree::community_row::row(main);
        crate::skin_tree::cover::glyphs(main, package)?;
        crate::skin_tree::label_style::worded(main, package)?;
    }
    let mut measured = crate::skin_tree::disabled::Measured::default();
    for layout in manifest.windows.values_mut() {
        crate::skin_tree::titles::menus(layout, package)?;
        crate::skin_tree::brand::nodes(layout);
        // Plan SK17: the chat title on its text point and the native
        // controls above the art, before the text boxes are laid.
        crate::skin_tree::chat_slots::serve(layout);
        crate::skin_tree::text_box::lay(layout);
        crate::skin_tree::inset::fields(layout);
        crate::skin_tree::target::nodes(layout);
        crate::skin_tree::disabled::nodes(layout, package, &mut measured)?;
    }
    // Plan SK17-6: the Community window wears the pack's own frame, the
    // served layout of its room window, else its chat, else its buddy list.
    crate::skin_tree::compose::community(manifest);
    Ok(measured.made())
}

/// Every `icons` key an icon slot and every `sounds` key a sound cue of
/// the registry.
fn check_keys(m: &SkinManifest) -> Result<(), SkinError> {
    if let Some(icon) = m.icons.keys().find(|k| IconSlot::of(k).is_none()) {
        return Err(refuse(SkinRule::UnknownIcon { icon: icon.clone() }));
    }
    if let Some(cue) = m.sounds.keys().find(|k| SoundCue::of(k).is_none()) {
        return Err(refuse(SkinRule::UnknownSoundCue { cue: cue.clone() }));
    }
    Ok(())
}

fn check_format(format: &str) -> Result<(), SkinError> {
    let expected = "rekindle-skin/<major>, the major a decimal integer";
    let digits = format
        .strip_prefix(SKIN_FORMAT_PREFIX)
        .filter(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
        .filter(|d| *d == "0" || !d.starts_with('0'))
        .ok_or_else(|| bad_value("format", expected))?;
    let major: u32 = digits.parse().map_err(|_| bad_value("format", expected))?;
    if major != SKIN_FORMAT_MAJOR {
        return Err(refuse(SkinRule::UnsupportedFormat { major }));
    }
    Ok(())
}

/// Lowercase ASCII letters, digits, `-` and `.`, starting with a letter or
/// digit, 1–64 bytes.
fn is_id(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= ID_MAX
        && b[0].is_ascii_alphanumeric()
        && b.iter()
            .all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-' || c == b'.')
}

fn is_lower_hex(s: &str, len: usize) -> bool {
    s.len() == len
        && s.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

fn check_fields(m: &SkinManifest) -> Result<(), SkinError> {
    let id_rule = "1-64 of a-z 0-9 . -, starting with a letter or digit";
    if !is_id(&m.id) {
        return Err(bad_value("id", id_rule));
    }
    text::check("name", &m.name, NAME).map_err(refuse)?;
    text::check("description", &m.description, DESCRIPTION).map_err(refuse)?;
    match &m.license {
        SkinLicense::Spdx(expr) => {
            spdx::Expression::parse_mode(expr, spdx::ParseMode::STRICT).map_err(|e| {
                refuse(SkinRule::BadSpdx {
                    detail: e.to_string(),
                })
            })?;
        }
        SkinLicense::Text(full) => text::check("license", full, LICENSE_TEXT).map_err(refuse)?,
    }
    if let Some(source) = &m.source {
        if !is_id(&source.format) {
            return Err(bad_value("source.format", id_rule));
        }
        if !is_lower_hex(&source.hash, BLAKE3_HEX) {
            return Err(bad_value(
                "source.hash",
                "a BLAKE3 hash, 64 lowercase hex digits",
            ));
        }
    }
    if let Some(credits) = &m.credits {
        check_credits(credits)?;
    }
    if let Some(author) = &m.author {
        if !is_lower_hex(author, ED25519_KEY_HEX) {
            return Err(bad_value(
                "author",
                "an Ed25519 public key, 64 lowercase hex digits",
            ));
        }
    }
    if let Some(signature) = &m.signature {
        if m.author.is_none() {
            return Err(bad_value("signature", "an author key beside it"));
        }
        if !is_lower_hex(signature, ED25519_SIGNATURE_HEX) {
            return Err(bad_value(
                "signature",
                "an Ed25519 signature, 128 lowercase hex digits",
            ));
        }
    }
    Ok(())
}

/// Each credited name as `name`, a bounded list, a printable-ASCII
/// `http(s)` URL (no space), the licence text as `license`'s.
fn check_credits(c: &SkinCredits) -> Result<(), SkinError> {
    for (field, names) in [
        ("credits.authors", &c.authors),
        ("credits.contributors", &c.contributors),
    ] {
        if names.len() > MAX_CREDIT_NAMES {
            return Err(bad_value(field, "at most 16 names"));
        }
        for (i, n) in names.iter().enumerate() {
            text::check(&format!("{field}[{i}]"), n, NAME).map_err(refuse)?;
        }
    }
    if let Some(url) = &c.url {
        let printable = url.bytes().all(|b| b.is_ascii_graphic());
        let scheme = url.starts_with("https://") || url.starts_with("http://");
        if !(printable && scheme && url.len() <= MAX_CREDIT_URL) {
            return Err(bad_value(
                "credits.url",
                "an http(s) URL of at most 512 printable ASCII characters",
            ));
        }
    }
    if let OriginalLicence::Text(full) = &c.licence {
        text::check("credits.licence", full, LICENSE_TEXT).map_err(refuse)?;
    }
    Ok(())
}

fn base_name(base: SkinBase) -> &'static str {
    match base {
        SkinBase::Light => "light",
        SkinBase::Dark => "dark",
    }
}

fn contrast_name(contrast: SkinContrast) -> &'static str {
    match contrast {
        SkinContrast::Standard => "standard",
        SkinContrast::High => "high",
    }
}

/// One or more variants, at most one per base and contrast, each naming a
/// root `*.json` token file (not the manifest) that the package holds.
fn check_variants(package: &Package<'_>, m: &SkinManifest) -> Result<(), SkinError> {
    if m.variants.is_empty() {
        return Err(refuse(SkinRule::NoVariants));
    }
    let mut seen = HashSet::new();
    for (i, v) in m.variants.iter().enumerate() {
        if !seen.insert((v.base, v.contrast)) {
            return Err(refuse(SkinRule::DuplicateVariant {
                base: base_name(v.base).to_owned(),
                contrast: contrast_name(v.contrast).to_owned(),
            }));
        }
        let field = format!("variants[{i}].tokens");
        if !is_root_json(&v.tokens) || v.tokens == MANIFEST {
            return Err(bad_value(
                &field,
                "a token file (*.json) at the package root",
            ));
        }
        if package.get(&v.tokens).is_none() {
            return Err(SkinError::new(
                v.tokens.as_str(),
                SkinRule::MissingEntry { field },
            ));
        }
    }
    Ok(())
}
