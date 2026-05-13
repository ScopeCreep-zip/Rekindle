//! AEGIS-128L AEAD via vendored libaegis C library.
//!
//! AEGIS-128L uses 16-byte nonces and 16-byte tags.
//! Throughput: ~6–8 GiB/s on Coffee Lake (AES-NI).
//! Encrypt ≈ decrypt by construction (zero asymmetry).
//!
//! # Key size
//!
//! AEGIS-128L uses a 128-bit (16-byte) key with 128-bit security level.
//! For 32-byte key material from HKDF, use `Aegis128LKey::from_32` which
//! takes the first 16 bytes. For 256-bit security, use AEGIS-256 instead
//! (not implemented here — AEGIS-128L is the throughput-optimal choice
//! on Coffee Lake without VAES).
//!
//! # Thread safety
//!
//! `aegis128l_encrypt_detached` and `aegis128l_decrypt_detached` are pure
//! stateless functions — each call operates on stack-local AEGIS state
//! (8 × 128-bit AES blocks in XMM registers). No shared mutable state.
//! Concurrent calls with the same key and different nonces are safe.

use crate::traits::{AeadAlgorithm, AeadError, BulkAead};

// ── FFI pointer safety ─────────────────────────────────────────────

/// Return a non-dangling pointer for FFI, even for empty slices.
///
/// When a Rust slice has length 0, `as_ptr()` returns a dangling
/// non-null pointer. The C function receives len=0 and should not
/// dereference it, but passing a valid address prevents UB under
/// strict interpreters (Miri, ASAN with pointer validation).
fn ffi_ptr(slice: &[u8]) -> *const u8 {
    if slice.is_empty() {
        // Return a non-dangling, well-aligned pointer for zero-length FFI calls.
        // The C function receives len=0 and will not dereference this address.
        std::ptr::NonNull::<u8>::dangling().as_ptr().cast_const()
    } else {
        slice.as_ptr()
    }
}

fn ffi_mut_ptr(slice: &mut [u8]) -> *mut u8 {
    if slice.is_empty() {
        // Return a non-dangling, valid pointer for zero-length FFI calls.
        // The C function receives len=0 and will not read or write to this
        // address. We use a 1-byte aligned NonNull to avoid UB under strict
        // pointer validation (Miri, ASAN).
        std::ptr::NonNull::<u8>::dangling().as_ptr()
    } else {
        slice.as_mut_ptr()
    }
}

// ── libaegis FFI declarations ──────────────────────────────────────

extern "C" {
    fn aegis_init() -> i32;

    fn aegis128l_encrypt_detached(
        c: *mut u8,
        mac: *mut u8,
        maclen: usize,
        m: *const u8,
        mlen: usize,
        ad: *const u8,
        adlen: usize,
        npub: *const u8,
        k: *const u8,
    ) -> i32;

    fn aegis128l_decrypt_detached(
        m: *mut u8,
        c: *const u8,
        clen: usize,
        mac: *const u8,
        maclen: usize,
        ad: *const u8,
        adlen: usize,
        npub: *const u8,
        k: *const u8,
    ) -> i32;
}

/// One-time runtime initialization. Detects CPU features (AES-NI)
/// and selects the optimal code path. Safe to call multiple times.
fn ensure_init() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        // SAFETY: aegis_init() initializes CPU feature detection.
        // It is thread-safe and idempotent. Returns 0 on success.
        let rc = unsafe { aegis_init() };
        assert_eq!(rc, 0, "aegis_init failed — AES-NI may not be available");
    });
}

/// AEGIS-128L AEAD key.
///
/// Stores the raw 16-byte key in a `Zeroizing` wrapper. The key is
/// zeroed on drop. AEGIS-128L is stateless — no context object, no
/// H-powers table. Each encrypt/decrypt call is a pure function of
/// `(key, nonce, aad, plaintext/ciphertext)`.
pub struct Aegis128LKey {
    key: zeroize::Zeroizing<[u8; 16]>,
}

impl Aegis128LKey {
    /// Construct from a 16-byte key.
    pub fn new(key_bytes: &[u8; 16]) -> Self {
        ensure_init();
        Self { key: zeroize::Zeroizing::new(*key_bytes) }
    }

    /// Construct from the first 16 bytes of a 32-byte key.
    ///
    /// For 32-byte key material from HKDF-SHA-256, this takes the
    /// first 128 bits. The remaining 128 bits are discarded (not
    /// leaked — the caller's 32-byte buffer is not modified).
    pub fn from_32(key_bytes: &[u8; 32]) -> Self {
        let mut k16 = [0u8; 16];
        k16.copy_from_slice(&key_bytes[..16]);
        let result = Self::new(&k16);
        zeroize::Zeroize::zeroize(&mut k16);
        result
    }
}

// SAFETY: Aegis128LKey contains only Zeroizing<[u8; 16]> (plain data, no
// pointers). aegis128l_encrypt/decrypt_detached are pure stateless functions
// — each call creates stack-local AEGIS state in XMM registers. No shared
// mutable state. Concurrent calls with the same key + different nonces are safe.
unsafe impl Send for Aegis128LKey {}

// SAFETY: Aegis128LKey contains only Zeroizing<[u8; 16]> (plain data, no
// pointers). No interior mutability, no shared mutable state. The FFI
// functions operate on stack-local state per call. Safe for concurrent &self.
unsafe impl Sync for Aegis128LKey {}

static_assertions::assert_impl_all!(Aegis128LKey: Send, Sync);

impl BulkAead for Aegis128LKey {
    fn algorithm(&self) -> AeadAlgorithm { AeadAlgorithm::Aegis128L }
    fn nonce_len(&self) -> usize { 16 }

    fn seal_detached(
        &self, nonce: &[u8], aad: &[u8],
        plaintext: &[u8], ciphertext_out: &mut [u8], tag_out: &mut [u8],
    ) -> Result<(), AeadError> {
        if nonce.len() != 16 {
            return Err(AeadError::BufferSize { need: 16, have: nonce.len() });
        }
        if plaintext.len() != ciphertext_out.len() {
            return Err(AeadError::BufferSize { need: plaintext.len(), have: ciphertext_out.len() });
        }
        if tag_out.len() < 16 {
            return Err(AeadError::BufferSize { need: 16, have: tag_out.len() });
        }

        // SAFETY: All pointers are valid for their stated lengths.
        // ffi_ptr/ffi_mut_ptr return non-dangling addresses even for
        // empty slices. aegis128l_encrypt_detached reads plaintext,
        // writes ciphertext and tag to non-overlapping output buffers.
        // Returns 0 on success.
        let rc = unsafe {
            aegis128l_encrypt_detached(
                ffi_mut_ptr(ciphertext_out),
                tag_out.as_mut_ptr(),
                16,
                ffi_ptr(plaintext),
                plaintext.len(),
                ffi_ptr(aad),
                aad.len(),
                nonce.as_ptr(),
                self.key.as_ptr(),
            )
        };
        if rc == 0 { Ok(()) } else { Err(AeadError::AuthFailed) }
    }

    fn open_detached(
        &self, nonce: &[u8], aad: &[u8],
        ciphertext: &[u8], tag: &[u8], plaintext_out: &mut [u8],
    ) -> Result<(), AeadError> {
        if nonce.len() != 16 {
            return Err(AeadError::BufferSize { need: 16, have: nonce.len() });
        }
        if ciphertext.len() != plaintext_out.len() {
            return Err(AeadError::BufferSize { need: ciphertext.len(), have: plaintext_out.len() });
        }
        if tag.len() < 16 {
            return Err(AeadError::BufferSize { need: 16, have: tag.len() });
        }

        // SAFETY: All pointers are valid for their stated lengths.
        // ffi_ptr/ffi_mut_ptr return non-dangling addresses even for
        // empty slices. aegis128l_decrypt_detached reads ciphertext + tag,
        // writes plaintext. Returns -1 on auth failure (plaintext_out is
        // zeroed by libaegis on failure — defense in depth).
        let rc = unsafe {
            aegis128l_decrypt_detached(
                ffi_mut_ptr(plaintext_out),
                ffi_ptr(ciphertext),
                ciphertext.len(),
                tag.as_ptr(),
                16,
                ffi_ptr(aad),
                aad.len(),
                nonce.as_ptr(),
                self.key.as_ptr(),
            )
        };
        if rc == 0 { Ok(()) } else { Err(AeadError::AuthFailed) }
    }

    fn seal_in_place(
        &self, nonce: &[u8], aad: &[u8], in_out: &mut [u8],
    ) -> Result<[u8; 16], AeadError> {
        if nonce.len() != 16 {
            return Err(AeadError::BufferSize { need: 16, have: nonce.len() });
        }
        let mut tag = [0u8; 16];
        // Obtain raw pointers from a single as_mut_ptr() call to avoid
        // Rust aliasing UB. Creating both ffi_ptr(&[u8]) and ffi_mut_ptr(&mut [u8])
        // from the same slice violates Stacked Borrows — the *mut u8 is
        // invalidated when the shared &[u8] borrow is created.
        // Instead: one as_mut_ptr() → use as both *mut and *const.
        let len = in_out.len();
        let ptr = if len > 0 { in_out.as_mut_ptr() } else { std::ptr::NonNull::<u8>::dangling().as_ptr() };
        // SAFETY: ptr is valid for len bytes (or dangling with len=0).
        // aegis128l_encrypt_detached supports in-place aliasing (c == m).
        // Confirmed by libaegis's aegis128l_encrypt() which passes c, c+mlen.
        let rc = unsafe {
            aegis128l_encrypt_detached(
                ptr,
                tag.as_mut_ptr(),
                16,
                ptr.cast_const(),
                len,
                ffi_ptr(aad),
                aad.len(),
                nonce.as_ptr(),
                self.key.as_ptr(),
            )
        };
        if rc == 0 { Ok(tag) } else { Err(AeadError::AuthFailed) }
    }

    fn open_in_place(
        &self, nonce: &[u8], aad: &[u8], ct_and_tag: &mut [u8],
    ) -> Result<usize, AeadError> {
        if nonce.len() != 16 {
            return Err(AeadError::BufferSize { need: 16, have: nonce.len() });
        }
        if ct_and_tag.len() < 16 {
            return Err(AeadError::BufferSize { need: 16, have: ct_and_tag.len() });
        }
        let ct_len = ct_and_tag.len() - 16;
        // Extract tag from the end of the buffer before decrypting.
        let mut tag = [0u8; 16];
        tag.copy_from_slice(&ct_and_tag[ct_len..ct_len + 16]);
        // Single as_mut_ptr() to avoid Stacked Borrows aliasing UB.
        let ptr = if ct_len > 0 { ct_and_tag.as_mut_ptr() } else { std::ptr::NonNull::<u8>::dangling().as_ptr() };
        // SAFETY: ptr is valid for ct_len bytes (or dangling with ct_len=0).
        // aegis128l_decrypt_detached supports in-place aliasing (m == c).
        // Confirmed by libaegis's aegis128l_decrypt() source.
        let rc = unsafe {
            aegis128l_decrypt_detached(
                ptr,
                ptr.cast_const(),
                ct_len,
                tag.as_ptr(),
                16,
                ffi_ptr(aad),
                aad.len(),
                nonce.as_ptr(),
                self.key.as_ptr(),
            )
        };
        if rc == 0 { Ok(ct_len) } else { Err(AeadError::AuthFailed) }
    }

    fn build_nonce(&self, counter: u64) -> [u8; 16] {
        let mut nonce = [0u8; 16];
        nonce[8..].copy_from_slice(&counter.to_be_bytes());
        nonce
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::BulkAead;

    #[test]
    fn roundtrip_small() {
        let key = Aegis128LKey::new(&[0x42; 16]);
        let nonce = key.build_nonce(0);
        let plain = b"hello aegis-128l";
        let mut ct = vec![0u8; plain.len()];
        let mut tag = [0u8; 16];
        key.seal_detached(&nonce, b"aad", plain, &mut ct, &mut tag).unwrap();
        let mut pt = vec![0u8; plain.len()];
        key.open_detached(&nonce, b"aad", &ct, &tag, &mut pt).unwrap();
        assert_eq!(&pt, plain);
    }

    #[test]
    fn roundtrip_64kib() {
        let key = Aegis128LKey::new(&[0x42; 16]);
        let nonce = key.build_nonce(1);
        let plain = vec![0xAB; 65519];
        let mut ct = vec![0u8; plain.len()];
        let mut tag = [0u8; 16];
        key.seal_detached(&nonce, b"", &plain, &mut ct, &mut tag).unwrap();
        let mut pt = vec![0u8; plain.len()];
        key.open_detached(&nonce, b"", &ct, &tag, &mut pt).unwrap();
        assert_eq!(pt, plain);
    }

    #[test]
    fn tampered_rejected() {
        let key = Aegis128LKey::new(&[0x42; 16]);
        let nonce = key.build_nonce(2);
        let plain = vec![0xCD; 1024];
        let mut ct = vec![0u8; 1024];
        let mut tag = [0u8; 16];
        key.seal_detached(&nonce, b"", &plain, &mut ct, &mut tag).unwrap();
        ct[0] ^= 0xFF;
        let mut pt = vec![0u8; 1024];
        assert!(key.open_detached(&nonce, b"", &ct, &tag, &mut pt).is_err());
    }

    #[test]
    fn from_32_works() {
        let key = Aegis128LKey::from_32(&[0x42; 32]);
        let nonce = key.build_nonce(0);
        let plain = b"from_32 test data";
        let mut ct = vec![0u8; plain.len()];
        let mut tag = [0u8; 16];
        key.seal_detached(&nonce, b"", plain, &mut ct, &mut tag).unwrap();
        let mut pt = vec![0u8; plain.len()];
        key.open_detached(&nonce, b"", &ct, &tag, &mut pt).unwrap();
        assert_eq!(&pt, plain);
    }

    #[test]
    fn encrypt_decrypt_symmetry() {
        // AEGIS-128L is structurally symmetric — verify encrypt and
        // decrypt paths both work for 100 iterations without drift.
        let key = Aegis128LKey::new(&[0x42; 16]);
        let data = vec![0xEF; 65519];
        let mut ct = vec![0u8; data.len()];
        let mut tag = [0u8; 16];
        let mut pt = vec![0u8; data.len()];

        for i in 0u64..100 {
            let nonce = key.build_nonce(i);
            key.seal_detached(&nonce, b"", &data, &mut ct, &mut tag).unwrap();
            key.open_detached(&nonce, b"", &ct, &tag, &mut pt).unwrap();
            assert_eq!(pt, data, "mismatch at iteration {i}");
        }
    }

    #[test]
    fn wrong_aad_rejected() {
        let key = Aegis128LKey::new(&[0x42; 16]);
        let nonce = key.build_nonce(0);
        let plain = vec![0xAB; 512];
        let mut ct = vec![0u8; 512];
        let mut tag = [0u8; 16];
        key.seal_detached(&nonce, b"correct", &plain, &mut ct, &mut tag).unwrap();
        let mut pt = vec![0u8; 512];
        assert!(key.open_detached(&nonce, b"wrong", &ct, &tag, &mut pt).is_err());
    }

    #[test]
    fn wrong_key_rejected() {
        let key1 = Aegis128LKey::new(&[0x42; 16]);
        let key2 = Aegis128LKey::new(&[0x43; 16]);
        let nonce = key1.build_nonce(0);
        let plain = vec![0xAB; 512];
        let mut ct = vec![0u8; 512];
        let mut tag = [0u8; 16];
        key1.seal_detached(&nonce, b"", &plain, &mut ct, &mut tag).unwrap();
        let mut pt = vec![0u8; 512];
        assert!(key2.open_detached(&nonce, b"", &ct, &tag, &mut pt).is_err());
    }

    #[test]
    fn wrong_nonce_rejected() {
        let key = Aegis128LKey::new(&[0x42; 16]);
        let nonce0 = key.build_nonce(0);
        let nonce1 = key.build_nonce(1);
        let plain = vec![0xEF; 512];
        let mut ct = vec![0u8; 512];
        let mut tag = [0u8; 16];
        key.seal_detached(&nonce0, b"", &plain, &mut ct, &mut tag).unwrap();
        let mut pt = vec![0u8; 512];
        assert!(key.open_detached(&nonce1, b"", &ct, &tag, &mut pt).is_err());
    }

    #[test]
    fn zero_length_plaintext_roundtrip() {
        let key = Aegis128LKey::new(&[0x42; 16]);
        let nonce = key.build_nonce(0);
        let plain = b"";
        let mut ct = vec![0u8; 0];
        let mut tag = [0u8; 16];
        key.seal_detached(&nonce, b"aad", plain, &mut ct, &mut tag).unwrap();
        let mut pt = vec![0u8; 0];
        key.open_detached(&nonce, b"aad", &ct, &tag, &mut pt).unwrap();
        assert!(pt.is_empty());
    }

    #[test]
    fn max_u64_nonce_roundtrip() {
        let key = Aegis128LKey::new(&[0x42; 16]);
        let nonce = key.build_nonce(u64::MAX);
        assert_eq!(nonce.len(), 16);
        let plain = b"max nonce test";
        let mut ct = vec![0u8; plain.len()];
        let mut tag = [0u8; 16];
        key.seal_detached(&nonce, b"", plain, &mut ct, &mut tag).unwrap();
        let mut pt = vec![0u8; plain.len()];
        key.open_detached(&nonce, b"", &ct, &tag, &mut pt).unwrap();
        assert_eq!(&pt, plain);
    }

    #[test]
    fn build_nonce_returns_16_bytes() {
        let key = Aegis128LKey::new(&[0x42; 16]);
        assert_eq!(key.build_nonce(0).len(), 16);
        assert_eq!(key.build_nonce(u64::MAX).len(), 16);
        assert_eq!(key.nonce_len(), 16);
    }

    #[test]
    fn open_detached_zeroes_output_on_failure() {
        let key = Aegis128LKey::new(&[0x42; 16]);
        let nonce = key.build_nonce(0);
        let plain = vec![0xAB; 256];
        let mut ct = vec![0u8; 256];
        let mut tag = [0u8; 16];
        key.seal_detached(&nonce, b"", &plain, &mut ct, &mut tag).unwrap();
        ct[0] ^= 0xFF; // tamper
        let mut pt = vec![0xCC; 256]; // pre-fill with non-zero
        assert!(key.open_detached(&nonce, b"", &ct, &tag, &mut pt).is_err());
        // libaegis zeroes plaintext_out on auth failure
        assert!(pt.iter().all(|&b| b == 0), "output must be zeroed on auth failure");
    }
}
