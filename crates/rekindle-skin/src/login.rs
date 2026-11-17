//! The device login skin's render-only copy (plan E7.7.2 "Storage"): the
//! one package readable before unlock, so it keeps nothing but what the
//! login window draws. Name, description, source, author key, signature
//! and every sound go (the login window plays none); the id and name become
//! [`LOGIN_ID`] and [`LOGIN_NAME`], the same for every copy (the manifest
//! requires a name); the result is re-packaged and re-hashed, so it cannot be
//! matched to the package it came from.

use std::collections::BTreeMap;

use rekindle_types::dto::skin::{SkinError, SkinRule};

use crate::output::{zip, Canonical};
use crate::package::{self, MANIFEST};

/// The id every stripped copy carries.
pub const LOGIN_ID: &str = "login";
/// The name every stripped copy carries.
pub const LOGIN_NAME: &str = "Login skin";

/// The render-only copy of the canonical package `bytes`.
///
/// # Errors
/// `bytes` is not a package with a valid manifest.
pub fn strip(bytes: &[u8]) -> Result<Canonical, SkinError> {
    let pkg = package::read(bytes)?;
    let mut manifest = crate::manifest::parse(&pkg)?;
    let sounds: Vec<String> = manifest.sounds.values().cloned().collect();
    LOGIN_ID.clone_into(&mut manifest.id);
    LOGIN_NAME.clone_into(&mut manifest.name);
    manifest.description.clear();
    manifest.source = None;
    manifest.author = None;
    manifest.signature = None;
    manifest.sounds.clear();
    let mut files: BTreeMap<String, Vec<u8>> = pkg
        .files()
        .filter(|(name, _)| *name != MANIFEST && !sounds.iter().any(|s| s == name))
        .map(|(name, data)| (name.to_owned(), data.to_vec()))
        .collect();
    let json = serde_json::to_vec(&manifest).map_err(|e| {
        SkinError::new(
            MANIFEST,
            SkinRule::ManifestSyntax {
                detail: e.to_string(),
            },
        )
    })?;
    files.insert(MANIFEST.to_owned(), json);
    let out = zip(&files)?;
    let hash = *blake3::hash(&out).as_bytes();
    Ok(Canonical { bytes: out, hash })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{builtin, validate};

    #[test]
    fn the_login_copy_keeps_no_identifying_field_or_sound_and_still_validates() {
        let source = validate::run(builtin::PACKS[0].bytes).unwrap();
        let stripped = strip(&source.bytes).unwrap();
        assert_ne!(stripped.hash, source.hash, "re-hashed");
        let pkg = package::read(&stripped.bytes).unwrap();
        let m = crate::manifest::parse(&pkg).unwrap();
        assert_eq!(m.id, LOGIN_ID);
        assert_eq!(m.name, LOGIN_NAME);
        assert!(m.description.is_empty());
        assert!(m.source.is_none() && m.author.is_none() && m.signature.is_none());
        assert!(m.sounds.is_empty());
        assert!(pkg.files().all(|(name, _)| !name.starts_with("sounds/")));
        // What the login window draws passes the same pipeline.
        let again = validate::run(&stripped.bytes).unwrap();
        assert_eq!(again.bytes, stripped.bytes, "a fixed point");
    }

    #[test]
    fn a_sound_bank_loses_every_cue() {
        let bank = builtin_bank();
        let source = package::read(bank).unwrap();
        assert!(source.files().any(|(n, _)| n.starts_with("sounds/")));
        let stripped = strip(bank).unwrap();
        let pkg = package::read(&stripped.bytes).unwrap();
        assert!(pkg.files().all(|(n, _)| !n.starts_with("sounds/")));
    }

    fn builtin_bank() -> &'static [u8] {
        include_bytes!("../builtin/defaults.rkskin")
    }
}
