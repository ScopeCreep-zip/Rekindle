//! Random identifier bytes, from one place.
//!
//! Two copies of this existed — `rekindle-channel`'s used
//! `rand::random()` and `src-tauri`'s used `OsRng::fill_bytes`. Both are
//! cryptographically sound sources, which is exactly why the difference
//! went unnoticed: nothing was wrong, there were just two answers to one
//! question, and `check-duplicate-bodies` cannot see it because the
//! bodies genuinely differ.
//!
//! `OsRng` is the surviving one. It reads OS entropy directly rather
//! than through a userspace generator, so there is no reseeding policy
//! to reason about at a call site that only wants an identifier.

use rand::RngCore;

/// 16 random bytes — the shape of a `ChannelId`, `CategoryId` or
/// `RoleId`.
#[must_use]
pub fn id_bytes_16() -> [u8; 16] {
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes
}

/// `len` random bytes, for nonces and salts.
#[must_use]
pub fn bytes(len: usize) -> Vec<u8> {
    let mut out = vec![0u8; len];
    rand::rngs::OsRng.fill_bytes(&mut out);
    out
}
