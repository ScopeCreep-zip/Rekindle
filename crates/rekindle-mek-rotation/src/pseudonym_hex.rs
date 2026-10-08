//! Hex <-> `PseudonymKey` conversion.
//!
//! `rotate.rs` and `distribute.rs` each carried a copy of the decode
//! half, differing only in local variable names. One implementation.

use rekindle_types::id::PseudonymKey;

/// Parse a 64-char hex string into a `PseudonymKey`.
///
/// Returns `None` for invalid hex or any length other than 32 bytes —
/// a pseudonym is an Ed25519 public key, so a short decode is not a
/// usable identity and must not be padded into one.
pub(crate) fn pseudonym_from_hex(hex_str: &str) -> Option<PseudonymKey> {
    let bytes = hex::decode(hex_str).ok()?;
    let arr: [u8; 32] = bytes.try_into().ok()?;
    Some(PseudonymKey(arr))
}

/// Render a `PseudonymKey` as lowercase hex.
pub(crate) fn pseudonym_hex(pseudonym: &PseudonymKey) -> String {
    hex::encode(pseudonym.0)
}
