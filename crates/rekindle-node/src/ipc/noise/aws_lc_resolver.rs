//! Custom snow CryptoResolver using aws-lc-rs for AES-GCM.
//!
//! # Why this exists
//!
//! Snow's `ring-accelerated` resolver allocates a `Vec<u8>` on EVERY
//! decrypt call. The `CipherAESGCM::decrypt` method's `out.len() <
//! ciphertext.len()` branch is ALWAYS taken because the Noise protocol
//! structurally guarantees `out` is sized for plaintext (N bytes) while
//! `ciphertext` is plaintext + tag (N + 16 bytes). This means every
//! control-plane decrypt does a heap allocation of ~65 KiB.
//!
//! This resolver eliminates that allocation by:
//! 1. Copying `ciphertext[..message_len]` to `out` (one memcpy, no alloc)
//! 2. Calling `LessSafeKey::open_separate_gather` with the tag as a
//!    separate slice — dispatches to `EVP_AEAD_CTX_open_gather` directly
//!
//! # Security
//!
//! - Nonces are big-endian per Noise spec §11.4 for AESGCM
//! - Key material is held in `LessSafeKey` which zeroizes on drop
//!   (via aws-lc-rs's `EVP_AEAD_CTX_cleanup` → `OPENSSL_free`)
//! - `open_separate_gather` zeroes `out` on auth failure (aws-lc behavior)
//! - No `indicator_check!` overhead (not FIPS-gated)

use aws_lc_rs::aead::{
    Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN,
};
use snow::resolvers::{CryptoResolver, DefaultResolver, FallbackResolver};
use snow::params::{CipherChoice, DHChoice, HashChoice};
use snow::types::{Cipher, Dh, Hash, Random};

const CIPHERKEYLEN: usize = 32;
const TAGLEN: usize = 16;

/// AES-256-GCM cipher for snow using aws-lc-rs LessSafeKey.
///
/// Key is `None` until `set()` is called during handshake `split()`.
/// After `set()`, the `LessSafeKey` holds the pre-expanded key schedule
/// and H-powers table for the session lifetime — zero per-call setup.
#[derive(Default)]
struct AwsLcAesGcm {
    key: Option<LessSafeKey>,
}

impl Cipher for AwsLcAesGcm {
    fn name(&self) -> &'static str { "AESGCM" }

    fn set(&mut self, key: &[u8; CIPHERKEYLEN]) {
        let unbound = UnboundKey::new(&AES_256_GCM, key)
            .expect("32-byte key is always valid for AES-256-GCM");
        self.key = Some(LessSafeKey::new(unbound));
    }

    fn encrypt(
        &self, nonce: u64, authtext: &[u8],
        plaintext: &[u8], out: &mut [u8],
    ) -> usize {
        let key = self.key.as_ref().expect("cipher key not set");

        // Noise spec §11.4: AESGCM nonce is big-endian u64 in bytes [4..12].
        let mut nonce_bytes = [0u8; NONCE_LEN];
        nonce_bytes[4..].copy_from_slice(&nonce.to_be_bytes());
        let n = Nonce::assume_unique_for_key(nonce_bytes);

        // Copy plaintext to output, then encrypt in-place.
        out[..plaintext.len()].copy_from_slice(plaintext);
        let tag = key
            .seal_in_place_separate_tag(n, Aad::from(authtext), &mut out[..plaintext.len()])
            .expect("AES-GCM seal failed");

        // Append tag after ciphertext.
        let tag_start = plaintext.len();
        out[tag_start..tag_start + TAGLEN].copy_from_slice(tag.as_ref());
        tag_start + TAGLEN
    }

    fn decrypt(
        &self, nonce: u64, authtext: &[u8],
        ciphertext: &[u8], out: &mut [u8],
    ) -> Result<usize, snow::Error> {
        let key = self.key.as_ref().expect("cipher key not set");

        let mut nonce_bytes = [0u8; NONCE_LEN];
        nonce_bytes[4..].copy_from_slice(&nonce.to_be_bytes());
        let n = Nonce::assume_unique_for_key(nonce_bytes);

        let message_len = ciphertext.len()
            .checked_sub(TAGLEN)
            .ok_or(snow::Error::Decrypt)?;

        // Split ciphertext from tag — NO allocation.
        let (ct_bytes, tag_bytes) = ciphertext.split_at(message_len);

        // Decrypt via open_separate_gather: reads ct_bytes + tag_bytes,
        // writes plaintext to out. Calls EVP_AEAD_CTX_open_gather directly.
        // On auth failure, out is zeroed by aws-lc (defense in depth).
        key.open_separate_gather(
            n,
            Aad::from(authtext),
            ct_bytes,
            tag_bytes,
            &mut out[..message_len],
        ).map_err(|_| snow::Error::Decrypt)?;

        Ok(message_len)
    }
}

// Compile-time assertion: AwsLcAesGcm must be Send + Sync because
// snow's Cipher trait requires Send + Sync. LessSafeKey is Send + Sync
// (backed by EVP_AEAD_CTX with no per-call mutable state after init).
// Option<LessSafeKey> derives Send + Sync from LessSafeKey.
static_assertions::assert_impl_all!(AwsLcAesGcm: Send, Sync);

/// AWS-LC crypto resolver for snow.
///
/// Handles AES-GCM only. DH (X25519), RNG, and Hash fall back to
/// `DefaultResolver` via `FallbackResolver`.
pub struct AwsLcResolver;

impl CryptoResolver for AwsLcResolver {
    fn resolve_rng(&self) -> Option<Box<dyn Random>> { None }
    fn resolve_dh(&self, _: &DHChoice) -> Option<Box<dyn Dh>> { None }
    fn resolve_hash(&self, _: &HashChoice) -> Option<Box<dyn Hash>> { None }

    fn resolve_cipher(&self, choice: &CipherChoice) -> Option<Box<dyn Cipher>> {
        match choice {
            CipherChoice::AESGCM => Some(Box::new(AwsLcAesGcm::default())),
            CipherChoice::ChaChaPoly => None, // falls back to DefaultResolver
        }
    }
}

/// Build a snow `Builder` using the aws-lc-rs resolver with `DefaultResolver` fallback.
///
/// DH (X25519) uses `DefaultResolver` (curve25519-dalek). AES-GCM uses
/// `AwsLcResolver` (aws-lc-rs `LessSafeKey`). Hash and RNG use defaults.
pub fn noise_builder(params: &str) -> snow::Builder<'static> {
    let params: snow::params::NoiseParams = params.parse()
        .expect("invalid Noise params string");
    snow::Builder::with_resolver(
        params,
        Box::new(FallbackResolver::new(
            Box::new(AwsLcResolver),
            Box::new(DefaultResolver),
        )),
    )
}
