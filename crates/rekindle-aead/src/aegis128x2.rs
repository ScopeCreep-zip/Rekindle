//! AEGIS-128X2 AEAD via vendored libaegis C library.
//!
//! AEGIS-128X2 uses 2× parallel AEGIS-128L lanes with 64-byte (512-bit)
//! input rate per update. Faster than AEGIS-128L even on AES-NI without
//! VAES; significantly faster with VAES+AVX2.
//!
//! Same key/nonce/tag sizes as AEGIS-128L: 16/16/16 bytes.
//! Wire-compatible key derivation — only the internal state differs.
//!
//! # Key size
//!
//! AEGIS-128X2 uses a 128-bit (16-byte) key. For 32-byte key material
//! from HKDF, use `from_32` which takes the first 16 bytes. The upper
//! 16 bytes are discarded — HKDF output is uniformly random, so the
//! first 128 bits have full entropy.
//!
//! # Thread safety
//!
//! `aegis128x2_encrypt_detached` and `aegis128x2_decrypt_detached` are
//! pure stateless functions — stack-local AEGIS state per call. No shared
//! mutable state. Concurrent calls with the same key + different nonces
//! are safe.

use crate::traits::{AeadAlgorithm, AeadError, BulkAead};

fn ffi_ptr(slice: &[u8]) -> *const u8 {
    if slice.is_empty() {
        std::ptr::NonNull::<u8>::dangling().as_ptr().cast_const()
    } else {
        slice.as_ptr()
    }
}

fn ffi_mut_ptr(slice: &mut [u8]) -> *mut u8 {
    if slice.is_empty() {
        std::ptr::NonNull::<u8>::dangling().as_ptr()
    } else {
        slice.as_mut_ptr()
    }
}

extern "C" {
    fn aegis128x2_encrypt_detached(
        c: *mut u8, mac: *mut u8, maclen: usize,
        m: *const u8, mlen: usize,
        ad: *const u8, adlen: usize,
        npub: *const u8, k: *const u8,
    ) -> i32;

    fn aegis128x2_decrypt_detached(
        m: *mut u8, c: *const u8, clen: usize,
        mac: *const u8, maclen: usize,
        ad: *const u8, adlen: usize,
        npub: *const u8, k: *const u8,
    ) -> i32;
}

fn ensure_init() {
    crate::aegis_init::ensure_init();
}

/// AEGIS-128X2 AEAD key.
pub struct Aegis128X2Key {
    key: zeroize::Zeroizing<[u8; 16]>,
}

impl Aegis128X2Key {
    pub fn new(key_bytes: &[u8; 16]) -> Self {
        ensure_init();
        Self { key: zeroize::Zeroizing::new(*key_bytes) }
    }

    /// Construct from the first 16 bytes of a 32-byte KDF output.
    /// The upper 16 bytes are discarded. HKDF output is uniformly
    /// random — the first 128 bits have full entropy.
    pub fn from_32(key_bytes: &[u8; 32]) -> Self {
        let mut k16 = [0u8; 16];
        k16.copy_from_slice(&key_bytes[..16]);
        let result = Self::new(&k16);
        zeroize::Zeroize::zeroize(&mut k16);
        result
    }
}

// SAFETY: Aegis128X2Key holds only a fixed-size byte array (the key).
// The FFI functions are stateless — they take the key by pointer, read it,
// and do not store any mutable state. No shared mutable state exists.
unsafe impl Send for Aegis128X2Key {}
// SAFETY: See Send rationale above — stateless FFI, no interior mutability.
unsafe impl Sync for Aegis128X2Key {}
static_assertions::assert_impl_all!(Aegis128X2Key: Send, Sync);

impl BulkAead for Aegis128X2Key {
    fn algorithm(&self) -> AeadAlgorithm { AeadAlgorithm::Aegis128X2 }
    fn nonce_len(&self) -> usize { 16 }

    fn seal_detached(
        &self, nonce: &[u8], aad: &[u8],
        plaintext: &[u8], ciphertext_out: &mut [u8], tag_out: &mut [u8],
    ) -> Result<(), AeadError> {
        if nonce.len() != 16 { return Err(AeadError::BufferSize { need: 16, have: nonce.len() }); }
        if plaintext.len() != ciphertext_out.len() { return Err(AeadError::BufferSize { need: plaintext.len(), have: ciphertext_out.len() }); }
        if tag_out.len() < 16 { return Err(AeadError::BufferSize { need: 16, have: tag_out.len() }); }
        // SAFETY: All pointers are valid, non-overlapping, and sized per the length arguments.
        // aegis128x2_encrypt_detached is a pure function with no global state.
        let rc = unsafe {
            aegis128x2_encrypt_detached(
                ffi_mut_ptr(ciphertext_out), tag_out.as_mut_ptr(), 16,
                ffi_ptr(plaintext), plaintext.len(),
                ffi_ptr(aad), aad.len(),
                nonce.as_ptr(), self.key.as_ptr(),
            )
        };
        if rc == 0 { Ok(()) } else { Err(AeadError::AuthFailed) }
    }

    fn open_detached(
        &self, nonce: &[u8], aad: &[u8],
        ciphertext: &[u8], tag: &[u8], plaintext_out: &mut [u8],
    ) -> Result<(), AeadError> {
        if nonce.len() != 16 { return Err(AeadError::BufferSize { need: 16, have: nonce.len() }); }
        if ciphertext.len() != plaintext_out.len() { return Err(AeadError::BufferSize { need: ciphertext.len(), have: plaintext_out.len() }); }
        if tag.len() < 16 { return Err(AeadError::BufferSize { need: 16, have: tag.len() }); }
        // SAFETY: All pointers are valid, non-overlapping, and sized per the length arguments.
        // aegis128x2_decrypt_detached is a pure function with no global state.
        let rc = unsafe {
            aegis128x2_decrypt_detached(
                ffi_mut_ptr(plaintext_out), ffi_ptr(ciphertext), ciphertext.len(),
                tag.as_ptr(), 16,
                ffi_ptr(aad), aad.len(),
                nonce.as_ptr(), self.key.as_ptr(),
            )
        };
        if rc == 0 { Ok(()) } else { Err(AeadError::AuthFailed) }
    }

    fn seal_in_place(
        &self, nonce: &[u8], aad: &[u8], in_out: &mut [u8],
    ) -> Result<[u8; 16], AeadError> {
        if nonce.len() != 16 { return Err(AeadError::BufferSize { need: 16, have: nonce.len() }); }
        let mut tag = [0u8; 16];
        let len = in_out.len();
        let ptr = if len > 0 { in_out.as_mut_ptr() } else { std::ptr::NonNull::<u8>::dangling().as_ptr() };
        // SAFETY: aegis128x2_enc loads src into registers before storing to dst,
        // so c==m aliasing is safe. Confirmed by libaegis aegis128x2_enc source.
        let rc = unsafe {
            aegis128x2_encrypt_detached(
                ptr, tag.as_mut_ptr(), 16,
                ptr.cast_const(), len,
                ffi_ptr(aad), aad.len(),
                nonce.as_ptr(), self.key.as_ptr(),
            )
        };
        if rc == 0 { Ok(tag) } else { Err(AeadError::AuthFailed) }
    }

    fn open_in_place(
        &self, nonce: &[u8], aad: &[u8], ct_and_tag: &mut [u8],
    ) -> Result<usize, AeadError> {
        if nonce.len() != 16 { return Err(AeadError::BufferSize { need: 16, have: nonce.len() }); }
        if ct_and_tag.len() < 16 { return Err(AeadError::BufferSize { need: 16, have: ct_and_tag.len() }); }
        let ct_len = ct_and_tag.len() - 16;
        let mut tag = [0u8; 16];
        tag.copy_from_slice(&ct_and_tag[ct_len..ct_len + 16]);
        let ptr = if ct_len > 0 { ct_and_tag.as_mut_ptr() } else { std::ptr::NonNull::<u8>::dangling().as_ptr() };
        // SAFETY: ptr aliases for src/dst is safe — aegis128x2_decrypt_detached
        // loads ciphertext into registers before writing plaintext.
        // All pointers are valid and sized per the length arguments.
        let rc = unsafe {
            aegis128x2_decrypt_detached(
                ptr, ptr.cast_const(), ct_len,
                tag.as_ptr(), 16,
                ffi_ptr(aad), aad.len(),
                nonce.as_ptr(), self.key.as_ptr(),
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
    fn roundtrip() {
        let key = Aegis128X2Key::new(&[0x42; 16]);
        let nonce = key.build_nonce(0);
        let plain = b"hello aegis-128x2";
        let mut ct = vec![0u8; plain.len()];
        let mut tag = [0u8; 16];
        key.seal_detached(&nonce, b"aad", plain, &mut ct, &mut tag).unwrap();
        let mut pt = vec![0u8; plain.len()];
        key.open_detached(&nonce, b"aad", &ct, &tag, &mut pt).unwrap();
        assert_eq!(&pt, plain);
    }

    #[test]
    fn tampered_rejected() {
        let key = Aegis128X2Key::new(&[0x42; 16]);
        let nonce = key.build_nonce(0);
        let mut ct = vec![0u8; 1024];
        let mut tag = [0u8; 16];
        key.seal_detached(&nonce, b"", &[0xAB; 1024], &mut ct, &mut tag).unwrap();
        ct[0] ^= 0xFF;
        let mut pt = vec![0u8; 1024];
        assert!(key.open_detached(&nonce, b"", &ct, &tag, &mut pt).is_err());
    }

    #[test]
    fn in_place_roundtrip() {
        let key = Aegis128X2Key::new(&[0x42; 16]);
        let nonce = key.build_nonce(0);
        let original = vec![0xAB; 65519];
        let mut buf = original.clone();
        let tag = key.seal_in_place(&nonce, b"", &mut buf).unwrap();
        let mut ct_tag = Vec::with_capacity(buf.len() + 16);
        ct_tag.extend_from_slice(&buf);
        ct_tag.extend_from_slice(&tag);
        let pt_len = key.open_in_place(&nonce, b"", &mut ct_tag).unwrap();
        assert_eq!(&ct_tag[..pt_len], &original);
    }

    #[test]
    fn from_32_works() {
        let key = Aegis128X2Key::from_32(&[0x42; 32]);
        let nonce = key.build_nonce(0);
        let plain = b"from_32 test";
        let mut ct = vec![0u8; plain.len()];
        let mut tag = [0u8; 16];
        key.seal_detached(&nonce, b"", plain, &mut ct, &mut tag).unwrap();
        let mut pt = vec![0u8; plain.len()];
        key.open_detached(&nonce, b"", &ct, &tag, &mut pt).unwrap();
        assert_eq!(&pt, plain);
    }

    #[test]
    fn algorithm_discriminant() {
        let key = Aegis128X2Key::new(&[0x42; 16]);
        assert_eq!(key.algorithm(), AeadAlgorithm::Aegis128X2);
        assert_eq!(key.nonce_len(), 16);
        assert_eq!(key.tag_len(), 16);
    }
}
