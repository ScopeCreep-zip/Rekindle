//! AEAD construction — seal/open frames with AAD bound to Envelope and Header.
//!
//! `FrameCipher` is the ONLY type that performs AEAD operations in v3.
//! It holds the algorithm, key, and direction_id together — consumers
//! cannot get the direction or algorithm wrong because both are baked
//! in at construction.
//!
//! Algorithm selection is config-driven at handshake time:
//! - Default: AEGIS-128X2 (highest throughput, ~12 GiB/s on AVX2)
//! - Fallback: AEGIS-128L (~8 GiB/s)
//! - Regulatory: AES-256-GCM (FIPS 140-3 / FedRAMP / NIST compliant, ~3.6 GiB/s)
//!
//! AEGIS is provided by rekindle-aead (vendored libaegis), not aws-lc-rs.
//! AES-256-GCM is provided by aws-lc-rs. All algorithms implement the
//! BulkAead trait — FrameCipher is algorithm-agnostic.
//!
//! AAD per RTI-SPEC-001 §10.4:
//!   Data Lane: Envelope (32 bytes) || Header (32 bytes) = 64 bytes
//!   Other Lanes: Envelope (32 bytes) only
//!
//! Nonce layout per RTI-SPEC-001 §10.4:
//!   [direction_id; 4][counter LE; 8] padded to algorithm nonce_len
//!   AES-256-GCM:   12 bytes (direction_id + counter fills exactly)
//!   AEGIS-128L/X2: 16 bytes (direction_id + counter + 4 zero pad)

use std::sync::Arc;

use crate::v3::wire::constants::{AEAD_TAG_LEN, ENVELOPE_LEN, STREAM_HEADER_LEN};

/// Errors from AEAD operations.
#[derive(Debug)]
pub enum AeadError {
    VerificationFailed,
    InputTooShort,
    InitFailed,
}

/// The single AEAD type for v3 frame encryption/decryption.
///
/// Holds algorithm + key + direction_id. Constructed once per direction
/// per session. Shared via `Arc<FrameCipher>` across tasks.
///
/// The direction_id and algorithm are baked in — callers pass only a
/// counter. This eliminates both direction confusion and algorithm
/// confusion at every call site.
pub struct FrameCipher {
    cipher: Arc<dyn rekindle_aead::BulkAead>,
    direction_id: [u8; 4],
}

impl Clone for FrameCipher {
    fn clone(&self) -> Self {
        Self {
            cipher: Arc::clone(&self.cipher),
            direction_id: self.direction_id,
        }
    }
}

impl FrameCipher {
    /// Construct from an already-initialized cipher and a direction.
    ///
    /// `cipher`: An `Arc<dyn BulkAead>` — `AesGcmKey`, `Aegis128LKey`,
    ///   or `Aegis128X2Key`. The caller chooses the algorithm at session
    ///   establishment based on negotiated capabilities.
    /// `direction_id`: `DIRECTION_ID_D2L` or `DIRECTION_ID_L2D`.
    pub fn new(cipher: Arc<dyn rekindle_aead::BulkAead>, direction_id: [u8; 4]) -> Self {
        Self { cipher, direction_id }
    }

    /// Convenience: construct from raw key bytes with AES-256-GCM.
    /// Used in tests and as the regulatory-compliance fallback.
    pub fn aes256gcm(key_bytes: &[u8; 32], direction_id: [u8; 4]) -> Result<Self, AeadError> {
        let key = rekindle_aead::aes_gcm::AesGcmKey::new(key_bytes)
            .map_err(|_| AeadError::InitFailed)?;
        Ok(Self {
            cipher: Arc::new(key),
            direction_id,
        })
    }

    /// Convenience: construct from raw key bytes with AEGIS-128X2 (default).
    /// 128-bit key derived from the first 16 bytes of the 32-byte HKDF output.
    pub fn aegis128x2(key_bytes: &[u8; 32], direction_id: [u8; 4]) -> Self {
        let key_16: [u8; 16] = key_bytes[..16].try_into().expect("slice is 16 bytes");
        let key = rekindle_aead::aegis128x2::Aegis128X2Key::new(&key_16);
        Self {
            cipher: Arc::new(key),
            direction_id,
        }
    }

    /// Build the nonce with direction_id prefix, padded to algorithm nonce_len.
    ///
    /// AES-256-GCM (nonce_len=12): [dir_id; 4][counter LE; 8]
    /// AEGIS-128L/X2 (nonce_len=16): [dir_id; 4][counter LE; 8][0; 4]
    ///
    /// The counter is stored as little-endian u64 bytes. The AEAD library
    /// and kernel GCM implementation read the 12-byte IV as big-endian
    /// internally (aesni-intel_glue.c reads bytes 4-7 and 8-11 as
    /// get_unaligned_be32), but this is transparent — both sides of the
    /// connection produce identical byte sequences, so the internal
    /// interpretation is irrelevant to correctness.
    fn build_nonce(&self, counter: u64) -> Vec<u8> {
        let nonce_len = self.cipher.nonce_len();
        let mut nonce = vec![0u8; nonce_len];
        let dir_len = std::cmp::min(4, nonce_len);
        nonce[..dir_len].copy_from_slice(&self.direction_id[..dir_len]);
        if nonce_len >= 12 {
            nonce[4..12].copy_from_slice(&counter.to_le_bytes());
        }
        nonce
    }

    /// Encrypt a plaintext frame payload, returning `ciphertext || tag`.
    ///
    /// `counter`: per-direction monotonic nonce counter.
    /// `envelope`: 32-byte Envelope bytes (always in AAD).
    /// `header`: 32-byte Stream Header for Data Lane (in AAD), None for other lanes.
    /// `plaintext`: frame payload to encrypt.
    pub fn seal(
        &self,
        counter: u64,
        envelope: &[u8; ENVELOPE_LEN],
        header: Option<&[u8; STREAM_HEADER_LEN]>,
        plaintext: &[u8],
    ) -> Vec<u8> {
        let aad = build_aad(envelope, header);
        let nonce = self.build_nonce(counter);

        let mut ct = vec![0u8; plaintext.len()];
        let mut tag = [0u8; AEAD_TAG_LEN];

        self.cipher.seal_detached(&nonce, &aad, plaintext, &mut ct, &mut tag)
            .expect("seal_detached failed — key or buffer invariant violated");

        let mut out = Vec::with_capacity(plaintext.len() + AEAD_TAG_LEN);
        out.extend_from_slice(&ct);
        out.extend_from_slice(&tag);
        out
    }

    /// Decrypt and verify a `ciphertext || tag` frame payload.
    ///
    /// Returns plaintext on success, `AeadError::VerificationFailed` on
    /// tampering, wrong key, wrong nonce, or wrong AAD.
    pub fn open(
        &self,
        counter: u64,
        envelope: &[u8; ENVELOPE_LEN],
        header: Option<&[u8; STREAM_HEADER_LEN]>,
        ciphertext_and_tag: &[u8],
    ) -> Result<Vec<u8>, AeadError> {
        if ciphertext_and_tag.len() < AEAD_TAG_LEN {
            return Err(AeadError::InputTooShort);
        }

        let aad = build_aad(envelope, header);
        let nonce = self.build_nonce(counter);

        let ct_len = ciphertext_and_tag.len() - AEAD_TAG_LEN;
        let ct = &ciphertext_and_tag[..ct_len];
        let tag = &ciphertext_and_tag[ct_len..];

        let mut plaintext = vec![0u8; ct_len];

        self.cipher.open_detached(&nonce, &aad, ct, tag, &mut plaintext)
            .map_err(|_| AeadError::VerificationFailed)?;

        Ok(plaintext)
    }

    /// Encrypt plaintext directly into a destination buffer at `offset`.
    /// Writes `plaintext.len()` bytes of ciphertext followed by 16-byte tag.
    /// Total bytes written: `plaintext.len() + AEAD_TAG_LEN`.
    /// The destination must have at least that many bytes available at `offset`.
    ///
    /// Zero allocation — the ciphertext and tag are written in-place into
    /// the caller's buffer.
    pub fn seal_into(
        &self,
        counter: u64,
        envelope: &[u8; ENVELOPE_LEN],
        header: Option<&[u8; STREAM_HEADER_LEN]>,
        plaintext: &[u8],
        dest: &mut [u8],
        offset: usize,
    ) {
        let aad = build_aad(envelope, header);
        let nonce = self.build_nonce(counter);

        let ct_end = offset + plaintext.len();
        let tag_end = ct_end + AEAD_TAG_LEN;
        assert!(tag_end <= dest.len(), "seal_into: destination too small");

        let (ct_region, tag_region) = dest[offset..tag_end].split_at_mut(plaintext.len());
        let mut tag = [0u8; AEAD_TAG_LEN];

        self.cipher.seal_detached(&nonce, &aad, plaintext, ct_region, &mut tag)
            .expect("seal_detached failed");

        tag_region.copy_from_slice(&tag);
    }

    /// Encrypt plaintext IN-PLACE within the buffer. The buffer contains
    /// plaintext on input and ciphertext on output (same length). Returns
    /// the 16-byte authentication tag separately.
    ///
    /// This is the hot path for the bulk pipeline: the rayon worker
    /// calls seal_in_place on the plaintext region, then appends the
    /// returned tag.
    pub fn seal_in_place(
        &self,
        counter: u64,
        envelope: &[u8; ENVELOPE_LEN],
        header: Option<&[u8; STREAM_HEADER_LEN]>,
        in_out: &mut [u8],
    ) -> [u8; AEAD_TAG_LEN] {
        let aad = build_aad(envelope, header);
        let nonce = self.build_nonce(counter);
        self.cipher.seal_in_place(&nonce, &aad, in_out)
            .expect("seal_in_place failed — key or buffer invariant violated")
    }

    /// Decrypt and verify into a caller-provided buffer. Returns plaintext
    /// length on success. The caller must ensure `dest` has at least
    /// `ciphertext_and_tag.len() - AEAD_TAG_LEN` bytes of capacity.
    ///
    /// Zero-allocation decrypt path when the caller provides a pre-allocated
    /// destination buffer.
    pub fn open_into(
        &self,
        counter: u64,
        envelope: &[u8; ENVELOPE_LEN],
        header: Option<&[u8; STREAM_HEADER_LEN]>,
        ciphertext_and_tag: &[u8],
        dest: &mut [u8],
    ) -> Result<usize, AeadError> {
        if ciphertext_and_tag.len() < AEAD_TAG_LEN {
            return Err(AeadError::InputTooShort);
        }

        let aad = build_aad(envelope, header);
        let nonce = self.build_nonce(counter);

        let ct_len = ciphertext_and_tag.len() - AEAD_TAG_LEN;
        let ct = &ciphertext_and_tag[..ct_len];
        let tag = &ciphertext_and_tag[ct_len..];

        assert!(dest.len() >= ct_len, "open_into: dest too small");

        self.cipher.open_detached(&nonce, &aad, ct, tag, &mut dest[..ct_len])
            .map_err(|_| AeadError::VerificationFailed)?;

        Ok(ct_len)
    }

    /// The direction_id this cipher was constructed with.
    pub fn direction_id(&self) -> [u8; 4] {
        self.direction_id
    }

    /// The algorithm this cipher uses.
    pub fn algorithm(&self) -> rekindle_aead::AeadAlgorithm {
        self.cipher.algorithm()
    }

    /// The underlying cipher, for cases where the caller needs
    /// direct access (e.g., parallel bulk encryption on rayon workers).
    pub fn inner(&self) -> &Arc<dyn rekindle_aead::BulkAead> {
        &self.cipher
    }
}

/// Build the AAD for a frame.
///
/// Header present (Data Lane): `envelope || header` (64 bytes).
/// Header absent (Control/Audit/Handoff): `envelope` only (32 bytes).
fn build_aad(
    envelope: &[u8; ENVELOPE_LEN],
    header: Option<&[u8; STREAM_HEADER_LEN]>,
) -> Vec<u8> {
    match header {
        Some(hdr) => {
            let mut aad = Vec::with_capacity(ENVELOPE_LEN + STREAM_HEADER_LEN);
            aad.extend_from_slice(envelope);
            aad.extend_from_slice(hdr);
            aad
        }
        None => {
            let mut aad = Vec::with_capacity(ENVELOPE_LEN);
            aad.extend_from_slice(envelope);
            aad
        }
    }
}

/// Compute body_len for a non-data frame without performing AEAD.
/// `plaintext_len + AEAD_TAG_LEN` — the ciphertext is always
/// plaintext_len bytes of ciphertext + 16-byte tag.
pub const fn non_data_body_len(plaintext_len: usize) -> usize {
    plaintext_len + AEAD_TAG_LEN
}

/// Compute body_len for a data frame without performing AEAD.
/// `STREAM_HEADER_LEN + plaintext_len + AEAD_TAG_LEN`.
pub const fn data_body_len(plaintext_len: usize) -> usize {
    STREAM_HEADER_LEN + plaintext_len + AEAD_TAG_LEN
}
