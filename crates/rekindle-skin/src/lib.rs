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
//! * [`images::validate`]: every picture decoded under limits and
//!   re-encoded to canonical PNG or GIF (validator step 4).
//! * [`sounds::validate`]: every sound cue decoded and re-encoded to PCM16
//!   WAV (validator step 5).
//! * [`access::validate`]: accessibility and anti-spoofing on a resolved
//!   variant, measured on composited colours (validator step 6).
//! * [`regions::validate`]: text over every region image (WCAG F83) on a
//!   resolved variant, and [`flash`], the WCAG 2.3.1 flash test step 4 runs
//!   on every animation.
//! * [`output`]: step 7, the BLAKE3-addressed canonical package and a
//!   variant's `tokens.css` body.
//! * [`validate::run`]: steps 1–7 in order, what the helper process runs.
//! * [`helper`]: the Host's way in: [`helper::validate`] runs the pipeline
//!   in `rekindle-skin-helper` (`src/bin/`), a separate, OS-capped process,
//!   under a wall-clock deadline.
//! * [`builtin`]: the embedded built-in packs, helper-validated on first
//!   use per process; [`builtin::DEFAULT`] is Symbiosis.
//! * [`import`]: legacy skin formats into a package that then passes
//!   [`validate::run`] (plan E7.7.3).
//! * [`registry`]: the closed token registry, and [`generate`], the
//!   artefacts `cargo xtask gen-skin-registry` writes from it.
//!
//! Tier 3: no Veilid, no Tauri, no `unsafe`, no file or network I/O; the
//! one process [`helper`] starts and its pipes are the only exception.
//!
//! [`SkinError`]: rekindle_types::dto::skin::SkinError

#![forbid(unsafe_code)]

pub mod access;
pub mod builtin;
mod colour;
pub mod conform;
pub mod draft;
pub mod flash;
pub mod generate;
pub mod helper;
pub mod images;
pub mod import;
pub mod login;
pub mod manifest;
pub mod output;
pub mod package;
pub mod regions;
pub mod registry;
pub mod role_tag;
pub mod serve;
#[cfg(test)]
mod serve_tests;
pub mod shape;
pub mod skin_tree;
pub mod sounds;
pub mod tokens;
pub mod validate;
#[cfg(test)]
mod work;

/// The package, image and sound bounds of the §6 limits ledger.
#[path = "lib_limits.rs"]
pub mod limits;
