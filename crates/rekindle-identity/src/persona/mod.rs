//! Layer 4 — Persona. Social presentation metadata.
//!
//! `DisplayName` is a signed claim. `KindDescriptor` is a self-asserted
//! label. Neither participates in identity, derivation, storage keying,
//! or authorization decisions.
//!
//! **Import restriction (D-13):** This module MUST NOT be imported from
//! `root/`, `grant/`, `trust/`, `session/`, or `vault_label/`. Enforced
//! by the `label_source_scan_no_persona_import` CI test.

pub mod display;
pub mod kind;

pub use display::DisplayName;
pub use kind::KindDescriptor;
