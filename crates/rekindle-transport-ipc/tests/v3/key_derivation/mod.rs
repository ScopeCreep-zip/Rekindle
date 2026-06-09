//! Key derivation label tests.
//!
//! These catch single-character HKDF label typos that silently
//! produce wrong keys (no error — EMAC just fails on every frame).

mod label_uniqueness;
mod label_values;
