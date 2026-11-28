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
#[path = "login_tests.rs"]
mod tests;
