//! The whole validator pipeline in one call (plan E7.7.1 "Validator
//! pipeline"): steps 1–7 in order, the first failure aborting with its
//! typed refusal, so nothing is partly applied.
//!
//! This is what the `rekindle-skin-helper` process runs on the bytes it
//! reads; the Host reaches it through [`crate::helper::validate`], never
//! in its own process.

use std::collections::BTreeMap;

use rekindle_types::dto::skin::{SkinError, SkinManifest, SkinRule, TokenValue};

use crate::images::PngEncoding;
use crate::output::{self, Canonical};
use crate::registry::{resolve, Base, TokenEntry, ENTRIES};
use crate::{access, images, manifest, package, regions, shape, sounds, tokens};

/// A variant whose token file the package lacks.
fn missing(tokens: &str) -> SkinError {
    SkinError::new(
        tokens,
        SkinRule::MissingEntry {
            field: "variants.tokens".to_owned(),
        },
    )
}

/// Validates the package `bytes` and returns its canonical package:
/// 1. the outer size and the strict zip walk; 2. the manifest and its
///    text rules; 3. every variant's tokens; 4. the images; 5. the sounds,
///    then each window's frame layout (plan SK0 S1);
///    6. accessibility, anti-spoofing and text over every region on each
///    variant after the defaults merge, and every status icon against the
///    rules its status colour meets; 7. the canonical output.
///
/// # Errors
/// The first rule the package breaks, naming the entry.
pub fn run(bytes: &[u8]) -> Result<Canonical, SkinError> {
    run_on(bytes, ENTRIES)
}

/// [`run`] with the tokens resolved and checked over `table` alone.
///
/// # Errors
/// The first rule the package breaks, naming the entry.
pub fn run_on(bytes: &[u8], table: &[TokenEntry]) -> Result<Canonical, SkinError> {
    let package = package::read(bytes)?;
    let (manifest, parsed) = parse(&package)?;
    let assets = Assets::check(&package, &manifest, PngEncoding::Canonical)?;
    check_colours(&manifest, &parsed, &assets, table)?;
    output::package(&manifest, &parsed, &assets.images, &assets.sounds)
}

/// Checks every candidate of one import's colour walk. The strict ZIP and
/// manifest walk and the token parser still run on every candidate.
#[derive(Default)]
pub(crate) struct ImportTrial {
    checked: Option<TrialAssets>,
}

struct TrialAssets {
    files: BTreeMap<String, Vec<u8>>,
    assets: Assets,
}

impl ImportTrial {
    pub(crate) fn check(&mut self, bytes: &[u8], table: &[TokenEntry]) -> Result<(), SkinError> {
        let package = package::read(bytes)?;
        let (manifest, parsed) = parse(&package)?;
        // The token names validated by manifest::parse are root JSON files.
        // Assets, cues and placed art use the manifest and the other files,
        // all of whose names and bytes must match before this reuse.
        let source = || {
            package
                .files()
                .filter(|(name, _)| !parsed.contains_key(*name))
        };
        let cached = self.checked.as_ref().filter(|checked| {
            source().eq(checked
                .files
                .iter()
                .map(|(name, bytes)| (name.as_str(), bytes.as_slice())))
        });
        if let Some(checked) = cached {
            return check_colours(&manifest, &parsed, &checked.assets, table);
        }
        let assets = Assets::check(&package, &manifest, PngEncoding::Trial)?;
        let result = check_colours(&manifest, &parsed, &assets, table);
        let files = source()
            .map(|(name, data)| (name.to_owned(), data.to_vec()))
            .collect();
        self.checked = Some(TrialAssets { files, assets });
        result
    }
}

type TokenFiles = BTreeMap<String, BTreeMap<String, TokenValue>>;

fn parse(package: &package::Package<'_>) -> Result<(SkinManifest, TokenFiles), SkinError> {
    let manifest = manifest::parse(package)?;
    let mut parsed = BTreeMap::new();
    for v in &manifest.variants {
        let file = package.get(&v.tokens).ok_or_else(|| missing(&v.tokens))?;
        parsed.insert(v.tokens.clone(), tokens::parse(&v.tokens, file)?);
    }
    Ok((manifest, parsed))
}

struct Assets {
    served: SkinManifest,
    images: BTreeMap<String, Vec<u8>>,
    sounds: BTreeMap<String, Vec<u8>>,
}

impl Assets {
    fn check(
        package: &package::Package<'_>,
        manifest: &SkinManifest,
        png: PngEncoding,
    ) -> Result<Self, SkinError> {
        let images = images::validate_with(package, manifest, png)?;
        let sounds = sounds::validate(package, manifest)?;
        // The served form is what is checked: a pack whose tree leaves no
        // room for the Host's placements is refused (design `sk2a` §3).
        // The Disabled copies are checked with the art they are drawn as.
        let mut served = manifest.clone();
        let mut drawn = images.clone();
        drawn.extend(manifest::place(&mut served, package)?);
        shape::check(&served, &drawn)?;
        Ok(Self {
            served,
            images,
            sounds,
        })
    }
}

fn check_colours(
    manifest: &SkinManifest,
    parsed: &TokenFiles,
    assets: &Assets,
    table: &[TokenEntry],
) -> Result<(), SkinError> {
    for v in &manifest.variants {
        let skin = parsed.get(&v.tokens).ok_or_else(|| missing(&v.tokens))?;
        let base = Base::of(v.base, v.contrast);
        let resolved = resolve(table, base, skin);
        access::validate(&v.tokens, base, &resolved)?;
        regions::validate(&assets.served, &assets.images, &resolved)?;
        access::icons::validate(&assets.served, &assets.images, base, &resolved)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "validate_tests.rs"]
mod tests;
