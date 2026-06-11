//! Sealed `Label` type — the ONLY way to address a vault key.
//!
//! Private field, no public constructor, no `FromStr`, no `From<String>`.
//! The ONLY way to get a `Label` is through the builder functions in
//! `builders.rs`. The vault API accepts `&Label` exclusively — inline
//! format strings cannot compile into vault keys.
//!
//! Grammar: `[a-z0-9.\-]{1,256}`. Full-width hex (64 chars for 32-byte
//! keys). Zero truncation anywhere in this module.

use crate::error::IdentityError;

/// A validated vault key label. Sealed: private field, no public constructor.
///
/// The `rekindle-storage` vault API signature changes to:
/// - `fn store_key(&self, label: &Label, …)`
/// - `fn load_key(&self, label: &Label)`
///
/// The old `&str` overloads are deleted. Every callsite that currently
/// constructs labels via `format!` or string manipulation is replaced
/// by a typed builder call.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Label(String);

impl Label {
    /// Crate-internal constructor. Called only by `builders.rs`.
    ///
    /// Validates the grammar and panics on violation — a violation
    /// here is always a crate-internal defect because builders
    /// construct labels from typed inputs with known structure.
    pub(crate) fn new_validated(label: String) -> Self {
        if let Err(e) = validate_grammar(&label) {
            panic!(
                "BUG: label builder produced invalid label '{label}': {e}. \
                 This is a code defect — the builder must produce valid labels."
            );
        }
        Self(label)
    }

    /// Read-only access to the label string.
    ///
    /// Used by `rekindle-storage` when passing to SQLCipher queries.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Display for Label {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

// Label is intentionally:
// - NOT FromStr (no parsing from arbitrary strings)
// - NOT From<String> / From<&str> (no construction from bare strings)
// - NOT Serialize / Deserialize (labels are not wire objects)
// - NOT Default (there is no meaningful default label)

/// Validate the label grammar: `[a-z0-9.\-]{1,256}`.
///
/// Internal — callers never need this because they go through builders.
fn validate_grammar(label: &str) -> Result<(), IdentityError> {
    if label.is_empty() || label.len() > 256 {
        return Err(IdentityError::LabelGrammar);
    }
    if !label.bytes().all(|b| {
        b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-'
    }) {
        return Err(IdentityError::LabelGrammar);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_labels() {
        Label::new_validated("origin.seed".into());
        Label::new_validated("prekey.spk.aabbccdd".into());
        Label::new_validated("a".into());
        Label::new_validated("a-b.c-d".into());
        Label::new_validated("a".repeat(256));
    }

    #[test]
    #[should_panic(expected = "BUG")]
    fn empty_label_panics() {
        Label::new_validated(String::new());
    }

    #[test]
    #[should_panic(expected = "BUG")]
    fn uppercase_panics() {
        Label::new_validated("UPPER.case".into());
    }

    #[test]
    #[should_panic(expected = "BUG")]
    fn colon_panics() {
        Label::new_validated("vld0:abc".into());
    }

    #[test]
    #[should_panic(expected = "BUG")]
    fn slash_panics() {
        Label::new_validated("foo/bar".into());
    }

    #[test]
    #[should_panic(expected = "BUG")]
    fn space_panics() {
        Label::new_validated("foo bar".into());
    }

    #[test]
    #[should_panic(expected = "BUG")]
    fn overlength_panics() {
        Label::new_validated("a".repeat(257));
    }

    #[test]
    fn as_str_roundtrip() {
        let label = Label::new_validated("origin.seed".into());
        assert_eq!(label.as_str(), "origin.seed");
    }

    #[test]
    fn display_matches_as_str() {
        let label = Label::new_validated("test.label".into());
        assert_eq!(format!("{label}"), "test.label");
    }

    #[test]
    fn equality() {
        let a = Label::new_validated("origin.seed".into());
        let b = Label::new_validated("origin.seed".into());
        let c = Label::new_validated("origin.other".into());
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn hashable() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(Label::new_validated("a".into()));
        set.insert(Label::new_validated("a".into()));
        set.insert(Label::new_validated("b".into()));
        assert_eq!(set.len(), 2);
    }
}
