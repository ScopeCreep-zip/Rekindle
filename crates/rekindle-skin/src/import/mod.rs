//! Legacy skin importers (plan E7.7.3): each reads a legacy tree held in
//! memory and builds a `rekindle-skin/1` package that then goes through
//! [`crate::validate::run`] like any other skin. Legacy archives are read
//! only here; nothing of them reaches a frontend but the canonical package.
//!
//! * [`xfire_xml`]: the Xfire XML era (Carbon Fiber, Symbiosis).
//! * [`walk`]: the minimal Oklab-lightness walk a built-in that fails the
//!   validator is adjusted by, every change recorded.
//!
//! The crate does no file I/O: the caller (`cargo xtask import-skin`, the
//! helper) hands the tree over as a map of relative paths to bytes.

mod tree;
pub mod walk;
pub mod xfire_xml;

use std::collections::BTreeMap;

use rekindle_types::dto::skin::SkinError;

use crate::output::Canonical;

/// A legacy tree: relative `/`-separated paths to file bytes.
pub type Tree = BTreeMap<String, Vec<u8>>;

/// Why an import stopped.
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("{file}: XML: {detail}")]
    Xml { file: String, detail: String },
    #[error("{file}: missing from the tree")]
    MissingFile { file: String },
    #[error("{name}: RGBA alpha {alpha} is not 0 (the byte is unused, COLORREF-style)")]
    NonZeroAlpha { name: String, alpha: i32 },
    #[error("{name}: {value} is not RGBA(r,g,b,a) with r, g, b in 0-255")]
    BadColour { name: String, value: String },
    #[error("{file}: image: {detail}")]
    Image { file: String, detail: String },
    #[error("the package is refused: {0}")]
    Refused(SkinError),
    #[error("{token}: no lightness passes {rule} on {ground}")]
    WalkExhausted {
        token: String,
        ground: String,
        rule: String,
    },
}

/// An imported skin: the canonical package and the `NOTICE` beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Imported {
    pub canonical: Canonical,
    pub notice: String,
    /// The walk's changes, in the order they were made.
    pub changes: Vec<walk::Change>,
}

/// The BLAKE3 of a tree, as the manifest's `source.hash`: every file in
/// path order, each as its path length, path, byte length and bytes.
#[must_use]
pub fn tree_hash(tree: &Tree) -> String {
    let mut h = blake3::Hasher::new();
    for (path, bytes) in tree {
        h.update(&(path.len() as u64).to_le_bytes());
        h.update(path.as_bytes());
        h.update(&(bytes.len() as u64).to_le_bytes());
        h.update(bytes);
    }
    h.finalize().to_hex().to_string()
}
