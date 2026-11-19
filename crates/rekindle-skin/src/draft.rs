//! The skin editor's checks (plan E7.7.5, Telegram's model): a draft is
//! one variant's tokens; the Host reports every declared pair's WCAG
//! ratio and pass/fail (APCA advisory), a suggested passing colour per
//! failing pair from the same minimal Oklab-lightness walk the importers
//! use ([`crate::import::walk`]), every other refusal the validator would
//! give, and the draft's token stylesheet for live apply. Nothing here is
//! applied for the author: a suggestion is a value the editor shows.
//!
//! Export builds the package [`validate::run`](crate::validate::run)
//! takes, so the editor's gate and the validator are one: a draft the
//! report calls exportable is a package the helper accepts. The package
//! states the author's licence and credits the author first, then what
//! the skin came from (owner 2026-11-18: everyone gets credit), and
//! carries the source's regions, icons and sounds with the regions the
//! author imported.

use std::collections::BTreeMap;

use rekindle_types::dto::skin::{
    OriginalLicence, SkinCredits, SkinError, SkinLicense, SkinManifest, SkinRule, SkinVariant,
    TokenValue, SKIN_FORMAT_MAJOR, SKIN_FORMAT_PREFIX,
};
use rekindle_types::dto::skin_draft::{
    SkinDraft, SkinDraftReport, SkinDraftToken, SkinPairCheck, SkinRegionSlot, SkinSuggestion,
};

#[path = "draft_assets.rs"]
mod assets;
pub use assets::Assets;

use crate::access::{self, apca, scaled, Measured};
use crate::import::walk::{color, lightness_walk, target_of};
use crate::output::{self, Canonical};
use crate::registry::{resolve, Base, Class, ENTRIES};
use crate::{colour, manifest, package, serve, tokens};

/// The draft's token file in an exported package.
pub const TOKENS: &str = "tokens.json";

/// The licence the draft states, read as the validator reads a manifest's:
/// a strict SPDX expression, otherwise the licence's full text.
#[must_use]
pub fn license_of(text: &str) -> SkinLicense {
    let trimmed = text.trim();
    if spdx::Expression::parse_mode(trimmed, spdx::ParseMode::STRICT).is_ok() {
        SkinLicense::Spdx(trimmed.to_owned())
    } else {
        SkinLicense::Text(text.to_owned())
    }
}

/// The credits the export carries: the draft's author first, then what
/// the skin came from (a derived skin credits its origin; owner
/// 2026-11-18). An empty author stays, so the validator names it.
#[must_use]
pub fn credits_of(draft: &SkinDraft) -> SkinCredits {
    let mut authors = vec![draft.author.clone()];
    authors.extend(
        draft
            .credits
            .authors
            .iter()
            .filter(|a| **a != draft.author)
            .cloned(),
    );
    SkinCredits {
        authors,
        ..draft.credits.clone()
    }
}

/// The draft's tokens checked as a package's token file is: an unknown
/// key, a fixed token or a value of the wrong type is refused.
///
/// # Errors
/// A token refusal.
pub fn parse_tokens(draft: &SkinDraft) -> Result<BTreeMap<String, TokenValue>, SkinError> {
    tokens::parse(TOKENS, &output::token_file(TOKENS, &draft.tokens)?)
}

/// The manifest id of a draft named `name`: lowercase ASCII letters and
/// digits, every other run as one `-`.
fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_end_matches('-');
    if trimmed.is_empty() {
        "draft".to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn manifest_of(draft: &SkinDraft, carried: assets::Carried) -> SkinManifest {
    SkinManifest {
        format: format!("{SKIN_FORMAT_PREFIX}{SKIN_FORMAT_MAJOR}"),
        id: slug(&draft.name),
        name: draft.name.clone(),
        description: draft.description.clone(),
        license: license_of(&draft.license),
        source: None,
        credits: Some(credits_of(draft)),
        author: None,
        signature: None,
        variants: vec![SkinVariant {
            base: draft.base,
            contrast: draft.contrast,
            tokens: TOKENS.to_owned(),
        }],
        regions: carried.regions,
        icons: carried.icons,
        sounds: carried.sounds,
    }
}

/// The package of `draft` with its assets: the source package's regions,
/// icons and sounds, and the regions the author imported. Not yet
/// validated: the helper runs [`crate::validate::run`] on these bytes.
///
/// # Errors
/// A token refusal, an unreadable source, an imported region without its
/// bytes, or a manifest that does not serialise.
pub fn package(draft: &SkinDraft, assets: &Assets) -> Result<Canonical, SkinError> {
    build(draft, Some(assets))
}

/// The package without assets (`None`: tokens and manifest text only), or
/// with them.
fn build(draft: &SkinDraft, assets: Option<&Assets>) -> Result<Canonical, SkinError> {
    let parsed = parse_tokens(draft)?;
    let tokens = BTreeMap::from([(TOKENS.to_owned(), parsed)]);
    let (carried, files) = match assets {
        Some(a) => assets::gather(draft, a)?,
        None => (assets::Carried::default(), BTreeMap::new()),
    };
    output::package(
        &manifest_of(draft, carried),
        &tokens,
        &files,
        &BTreeMap::new(),
    )
}

/// "Start from the current skin": the active variant of the canonical
/// package `id` as a draft. Its credits carry forward (the source's
/// authors, contributors and URL, and the licence it was made under), its
/// licence is offered as the draft's, and its regions, icons and sounds
/// stay with it through `from`.
///
/// # Errors
/// A package that does not read (never for one the pipeline returned).
pub fn from_package(id: &str, bytes: &[u8]) -> Result<SkinDraft, SkinError> {
    let read = package::read(bytes)?;
    let m = manifest::parse(&read)?;
    let v = serve::active(&m.variants).ok_or_else(|| {
        SkinError::new(
            SkinError::PACKAGE,
            SkinRule::MissingEntry {
                field: "variants".to_owned(),
            },
        )
    })?;
    let file = read.get(&v.tokens).ok_or_else(|| {
        SkinError::new(
            v.tokens.as_str(),
            SkinRule::MissingEntry {
                field: "variants.tokens".to_owned(),
            },
        )
    })?;
    let license = match &m.license {
        SkinLicense::Spdx(s) | SkinLicense::Text(s) => s.clone(),
    };
    let carried = m.credits.clone().unwrap_or(SkinCredits {
        authors: Vec::new(),
        contributors: Vec::new(),
        url: None,
        licence: OriginalLicence::NoneStated,
    });
    let licence = match carried.licence {
        OriginalLicence::Text(t) => OriginalLicence::Text(t),
        OriginalLicence::NoneStated => OriginalLicence::Text(license.clone()),
    };
    Ok(SkinDraft {
        name: m.name,
        description: m.description,
        author: String::new(),
        license,
        credits: SkinCredits { licence, ..carried },
        base: v.base,
        contrast: v.contrast,
        tokens: tokens::parse(&v.tokens, file)?,
        from: Some(id.to_owned()),
        regions: BTreeMap::new(),
    })
}

fn same(a: &Measured, b: &Measured) -> bool {
    a.rule == b.rule && a.fg == b.fg && a.ground == b.ground
}

/// `m` re-measured with `token` set to `rgb`.
fn remeasured(
    base: Base,
    tokens: &BTreeMap<String, TokenValue>,
    m: &Measured,
    token: &str,
    rgb: [u8; 3],
) -> Option<Measured> {
    let mut trial = tokens.clone();
    trial.insert(token.to_owned(), color(rgb));
    access::measured(base, &resolve(ENTRIES, base, &trial))
        .into_iter()
        .find(|x| same(x, m))
}

/// The smallest Oklab-lightness move of the colour the skin may set (the
/// text, else its ground) that passes `m`, away from the other colour.
fn suggestion(
    base: Base,
    tokens: &BTreeMap<String, TokenValue>,
    resolved: &BTreeMap<&str, [u8; 3]>,
    m: &Measured,
) -> Option<SkinSuggestion> {
    let (token, _) = target_of(&m.fg, &m.ground)?;
    let original = *resolved.get(token.as_str())?;
    let (moved, other) = if token == m.fg {
        (m.fg_rgb, m.ground_rgb)
    } else {
        (m.ground_rgb, m.fg_rgb)
    };
    let up = colour::luminance(moved) >= colour::luminance(other);
    let passes = |c| remeasured(base, tokens, m, &token, c).is_some_and(|x| x.passes());
    let rgb = lightness_walk(original, up, |c| Ok(passes(c))).ok()??;
    let ratio = remeasured(base, tokens, m, &token, rgb)?.ratio;
    Some(SkinSuggestion {
        token,
        rgb,
        ratio_x100: scaled(ratio, 100.0),
    })
}

fn apca_x10(m: &Measured) -> i32 {
    let lc = (apca::lc(m.fg_rgb, m.ground_rgb) * 10.0).round();
    // |Lc| ≤ 108 by construction, so the tenths fit.
    format!("{lc:.0}").parse().unwrap_or(0)
}

/// The Host's report on `draft` (`SkinValidateDraft`).
///
/// # Errors
/// A token refusal: the draft names a key the registry lacks, a fixed
/// token, or a value of the wrong type.
pub fn report(draft: &SkinDraft) -> Result<SkinDraftReport, SkinError> {
    let tokens = parse_tokens(draft)?;
    let base = Base::of(draft.base, draft.contrast);
    let resolved = resolve(ENTRIES, base, &tokens);
    let rgb: BTreeMap<&str, [u8; 3]> = resolved
        .iter()
        .filter_map(|(e, v)| match v {
            TokenValue::Color { r, g, b, .. } => Some((e.id, [*r, *g, *b])),
            _ => None,
        })
        .collect();
    let pairs: Vec<SkinPairCheck> = access::measured(base, &resolved)
        .into_iter()
        .map(|m| SkinPairCheck {
            ratio_x100: scaled(m.ratio, 100.0),
            required_x100: u32::from(m.required_x100),
            pass: m.passes(),
            apca_lc_x10: apca_x10(&m),
            suggestion: (!m.passes())
                .then(|| suggestion(base, &tokens, &rgb, &m))
                .flatten(),
            rule: m.rule,
            fg: m.fg,
            ground: m.ground,
        })
        .collect();
    let mut failures: Vec<String> = access::failures(base, &resolved)
        .into_iter()
        .filter(|r| !matches!(r, SkinRule::Contrast { .. }))
        .map(|r| r.to_string())
        .collect();
    let built = build(draft, None)?;
    if let Err(e) = package::read(&built.bytes).and_then(|p| manifest::parse(&p).map(drop)) {
        failures.push(e.to_string());
    }
    let tokens_css = output::tokens_css(&resolved).ok_or_else(|| {
        SkinError::new(
            TOKENS,
            SkinRule::TokenSyntax {
                detail: "the tokens do not emit".to_owned(),
            },
        )
    })?;
    let exportable = failures.is_empty() && pairs.iter().all(|p| p.pass);
    let listed = resolved
        .iter()
        .filter(|(e, _)| e.class == Class::Themeable)
        .map(|(e, v)| SkinDraftToken {
            id: e.id.to_owned(),
            css: e.css.to_owned(),
            value: v.clone(),
            set: tokens.contains_key(e.id),
        })
        .collect();
    let region_slots = crate::regions::SLOTS
        .iter()
        .map(|s| SkinRegionSlot {
            id: s.id.to_owned(),
            carries_text: !s.text.is_empty(),
        })
        .collect();
    Ok(SkinDraftReport {
        tokens: listed,
        pairs,
        failures,
        tokens_css,
        exportable,
        license: license_of(&draft.license),
        credits: credits_of(draft),
        region_slots,
    })
}

#[cfg(test)]
#[path = "draft_tests.rs"]
mod tests;
