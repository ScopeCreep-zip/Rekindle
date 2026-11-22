//! The whole validator pipeline in one call (plan E7.7.1 "Validator
//! pipeline"): steps 1–7 in order, the first failure aborting with its
//! typed refusal, so nothing is partly applied.
//!
//! This is what the `rekindle-skin-helper` process runs on the bytes it
//! reads; the Host reaches it through [`crate::helper::validate`], never
//! in its own process.

use std::collections::BTreeMap;

use rekindle_types::dto::skin::{SkinError, SkinRule};

use crate::output::{self, Canonical};
use crate::registry::{resolve, Base, ENTRIES};
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
    let package = package::read(bytes)?;
    let manifest = manifest::parse(&package)?;
    let mut parsed = BTreeMap::new();
    for v in &manifest.variants {
        let file = package.get(&v.tokens).ok_or_else(|| missing(&v.tokens))?;
        parsed.insert(v.tokens.clone(), tokens::parse(&v.tokens, file)?);
    }
    let images = images::validate(&package, &manifest)?;
    let sounds = sounds::validate(&package, &manifest)?;
    // The served form is what is checked: a pack whose tree leaves no room
    // for the Host's placements is refused (design `sk2a` §3). What is
    // written is the author's manifest, so the package stays its own.
    let mut served = manifest.clone();
    manifest::place(&mut served);
    shape::check(&served, &images)?;
    for v in &manifest.variants {
        let skin = parsed.get(&v.tokens).ok_or_else(|| missing(&v.tokens))?;
        let base = Base::of(v.base, v.contrast);
        let resolved = resolve(ENTRIES, base, skin);
        access::validate(&v.tokens, base, &resolved)?;
        regions::validate(&served, &images, &resolved)?;
        access::icons::validate(&served, &images, base, &resolved)?;
    }
    output::package(&manifest, &parsed, &images, &sounds)
}
