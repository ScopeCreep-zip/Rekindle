//! AES-256-GCM via `aws_lc_rs::aead::LessSafeKey`.
//!
//! `LessSafeKey` is the correct primitive for ratchet message keys:
//! each message key is used exactly once with a counter-derived nonce,
//! then discarded. The `BoundKey` family's `NonceSequence` ownership
//! model does not compose with single-use keys.

use aws_lc_rs::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};
use zeroize::Zeroizing;

use crate::error::RatchetError;

pub const TAG_LEN: usize = 16;

/// Build a `LessSafeKey` from a 32-byte message key.
pub fn build_key(mk: &Zeroizing<[u8; 32]>) -> Result<LessSafeKey, RatchetError> {
    let unbound =
        UnboundKey::new(&AES_256_GCM, mk.as_ref()).map_err(|_| RatchetError::AeadKey)?;
    Ok(LessSafeKey::new(unbound))
}

/// 96-bit nonce from a 32-bit counter: `[0u32 BE || counter BE || 0u32 BE]`.
///
/// Structurally non-repeating per chain: the counter increments monotonically,
/// and every DH ratchet step produces a new chain key (new AEAD key), so
/// counter reset is safe.
pub fn nonce_from_counter(counter: u32) -> Nonce {
    let mut n = [0u8; NONCE_LEN];
    n[4..8].copy_from_slice(&counter.to_be_bytes());
    Nonce::assume_unique_for_key(n)
}

/// Encrypt in place, appending the 16-byte GCM tag.
pub fn seal(
    key: &LessSafeKey,
    counter: u32,
    ad: &[u8],
    in_out: &mut Vec<u8>,
) -> Result<(), RatchetError> {
    let nonce = nonce_from_counter(counter);
    key.seal_in_place_append_tag(nonce, Aad::from(ad), in_out)
        .map_err(|_| RatchetError::AeadKey)
}

/// Decrypt in place. Input is `ciphertext || tag`. Returns plaintext slice.
///
/// The returned slice borrows `in_out` — caller must consume or copy
/// before the buffer is reused or zeroized.
pub fn open<'a>(
    key: &LessSafeKey,
    counter: u32,
    ad: &[u8],
    in_out: &'a mut [u8],
) -> Result<&'a mut [u8], RatchetError> {
    if in_out.len() < TAG_LEN {
        return Err(RatchetError::AeadBufferShort);
    }
    let nonce = nonce_from_counter(counter);
    key.open_in_place(nonce, Aad::from(ad), in_out)
        .map_err(|_| RatchetError::AeadOpen)
}
