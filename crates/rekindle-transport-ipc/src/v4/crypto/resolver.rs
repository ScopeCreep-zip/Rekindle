//! Custom snow CryptoResolver using aws-lc-rs for AES-GCM.
//!
//! Snow's ring resolver allocates a Vec<u8> on EVERY decrypt because
//! the `out.len() < ciphertext.len()` branch is structurally always
//! taken (Noise guarantees out is sized for plaintext while ciphertext
//! includes the 16-byte tag).
//!
//! This resolver eliminates that allocation via `open_separate_gather`
//! which dispatches to `EVP_AEAD_CTX_open_gather` — zero-alloc decrypt.
//!
//! Security:
//! - Nonces are big-endian per Noise spec section 11.4 for AESGCM
//! - LessSafeKey zeroizes on drop (EVP_AEAD_CTX_cleanup -> OPENSSL_free)
//! - open_separate_gather zeroes `out` on auth failure

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
/// After `set()`, LessSafeKey holds pre-expanded key schedule and
/// H-powers table — zero per-call setup.
#[derive(Default)]
struct AwsLcAesGcm {
    key: Option<LessSafeKey>,
}

impl Cipher for AwsLcAesGcm {
    fn name(&self) -> &'static str {
        "AESGCM"
    }

    fn set(&mut self, key: &[u8; CIPHERKEYLEN]) {
        let unbound = UnboundKey::new(&AES_256_GCM, key)
            .expect("32-byte key is always valid for AES-256-GCM");
        self.key = Some(LessSafeKey::new(unbound));
    }

    fn encrypt(
        &self,
        nonce: u64,
        authtext: &[u8],
        plaintext: &[u8],
        out: &mut [u8],
    ) -> usize {
        let key = self.key.as_ref().expect("cipher key not set");

        // Noise spec section 11.4: AESGCM nonce is big-endian u64 in bytes [4..12].
        let mut nonce_bytes = [0u8; NONCE_LEN];
        nonce_bytes[4..].copy_from_slice(&nonce.to_be_bytes());
        let n = Nonce::assume_unique_for_key(nonce_bytes);

        out[..plaintext.len()].copy_from_slice(plaintext);
        let tag = key
            .seal_in_place_separate_tag(n, Aad::from(authtext), &mut out[..plaintext.len()])
            .expect("AES-GCM seal failed");

        out[plaintext.len()..plaintext.len() + TAGLEN].copy_from_slice(tag.as_ref());
        plaintext.len() + TAGLEN
    }

    fn decrypt(
        &self,
        nonce: u64,
        authtext: &[u8],
        ciphertext: &[u8],
        out: &mut [u8],
    ) -> Result<usize, snow::Error> {
        let key = self.key.as_ref().expect("cipher key not set");

        let mut nonce_bytes = [0u8; NONCE_LEN];
        nonce_bytes[4..].copy_from_slice(&nonce.to_be_bytes());
        let n = Nonce::assume_unique_for_key(nonce_bytes);

        let message_len = ciphertext
            .len()
            .checked_sub(TAGLEN)
            .ok_or(snow::Error::Decrypt)?;

        // Split ciphertext from tag — NO allocation.
        let (ct_bytes, tag_bytes) = ciphertext.split_at(message_len);

        // open_separate_gather: reads ct + tag, writes plaintext to out.
        // Calls EVP_AEAD_CTX_open_gather directly.
        // On auth failure, out is zeroed by aws-lc (defense in depth).
        key.open_separate_gather(
            n,
            Aad::from(authtext),
            ct_bytes,
            tag_bytes,
            &mut out[..message_len],
        )
        .map_err(|_| snow::Error::Decrypt)?;

        Ok(message_len)
    }
}

// LessSafeKey is Send + Sync (backed by EVP_AEAD_CTX with no
// per-call mutable state after init).
static_assertions::assert_impl_all!(AwsLcAesGcm: Send, Sync);

/// AWS-LC crypto resolver for snow.
///
/// Handles AES-GCM only. DH (X25519), RNG, and Hash fall back to
/// DefaultResolver via FallbackResolver.
pub struct AwsLcResolver;

impl CryptoResolver for AwsLcResolver {
    fn resolve_rng(&self) -> Option<Box<dyn Random>> {
        None
    }
    fn resolve_dh(&self, _: &DHChoice) -> Option<Box<dyn Dh>> {
        None
    }
    fn resolve_hash(&self, _: &HashChoice) -> Option<Box<dyn Hash>> {
        None
    }
    fn resolve_cipher(&self, choice: &CipherChoice) -> Option<Box<dyn Cipher>> {
        match choice {
            CipherChoice::AESGCM => Some(Box::new(AwsLcAesGcm::default())),
            CipherChoice::ChaChaPoly => None, // falls back to DefaultResolver
        }
    }
}

/// Build a snow Builder with the aws-lc resolver + DefaultResolver fallback.
pub fn noise_builder(params: &str) -> snow::Builder<'static> {
    let params: snow::params::NoiseParams = params.parse().expect("invalid Noise params string");
    snow::Builder::with_resolver(
        params,
        Box::new(FallbackResolver::new(
            Box::new(AwsLcResolver),
            Box::new(DefaultResolver),
        )),
    )
}
