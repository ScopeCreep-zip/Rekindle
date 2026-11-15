//! The skin package format `rekindle-skin/1` (plan E7.7.1;
//! `.claude/plans/skins/01-recommendation.md` §2).
//!
//! A skin is a zip of declarative primitives (typed tokens, bitmap regions,
//! icons, sound cues, variants) and nothing a webview runs. Every skin, the
//! built-in ones included, passes this crate before a byte reaches a
//! frontend; a refusal is a [`SkinError`] naming the entry and the rule, and
//! nothing is partly applied.
//!
//! * [`package::read`]: the outer size and the strict in-memory zip walk.
//! * [`manifest::parse`]: `manifest.json` and its text rules.
//! * [`tokens::parse`]: a variant's token file (restricted DTCG 2025.10),
//!   checked against [`registry`], and [`tokens::emit`], the numeric
//!   re-emitter.
//! * [`registry`]: the closed token registry, and [`generate`], the
//!   artefacts `cargo xtask gen-skin-registry` writes from it.
//!
//! Tier 3: no Veilid, no Tauri, no `unsafe`, no file or network I/O.
//!
//! [`SkinError`]: rekindle_types::dto::skin::SkinError

#![forbid(unsafe_code)]

mod colour;
pub mod generate;
pub mod manifest;
pub mod package;
pub mod registry;
pub mod tokens;

/// The package bounds of the §6 limits ledger.
pub mod limits {
    /// A whole package.
    pub const MAX_PACKAGE_BYTES: u64 = 16 * 1024 * 1024;
    /// Zip entries, directories included.
    pub const MAX_ENTRIES: u64 = 512;
    /// `manifest.json`.
    pub const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
}
