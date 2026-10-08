//! SFrame (RFC 9605) for real-time media, cipher suite 0x0005
//! `AES_256_GCM_SHA512_128` only.
//!
//! The primitive (§4.3 header, §4.4.2 key derivation, §4.4.3 / §4.4.4
//! seal and open) plus Rekindle's sender keying, which follows the §5.1
//! "sender keys" scheme: every sender encrypts under its own base key,
//! derived here from the scope secret everyone in the call or channel
//! holds, the sender's signing key, and a random per-session tag carried
//! in the KID. Receivers identify the sender from the packet's signed
//! sender key (the §5.1 "signal outside of SFrame"), so no shared member
//! ordering is needed, and a long-lived scope secret (a channel MEK)
//! never repeats a `(base_key, CTR)` pair across sessions (§9.1).

use std::sync::atomic::{AtomicU64, Ordering};

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use hkdf::Hkdf;
use rekindle_types::domains::VOICE_SENDER_KEY;
use sha2::Sha512;
use zeroize::Zeroizing;

/// Cipher suite `AES_256_GCM_SHA512_128` (RFC 9605 §8.1).
pub const SUITE_AES_256_GCM_SHA512_128: u16 = 0x0005;
/// `AEAD.Nk` for the suite.
const NK: usize = 32;
/// `AEAD.Nn` for the suite.
const NN: usize = 12;
/// Longest header: config byte, 8-byte KID, 8-byte CTR (§4.3).
pub const MAX_HEADER_LEN: usize = 17;
/// Bits of the KID that carry the key generation.
const GENERATION_BITS: u32 = 8;
/// Mask for the sender tag (the KID bits above the generation).
const TAG_MASK: u64 = (1 << (64 - GENERATION_BITS)) - 1;

/// An SFrame key ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Kid(pub u64);

/// Why an SFrame operation failed. Callers drop the frame for every
/// variant; the distinction is for tests and counters only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SframeError {
    /// The header is truncated.
    Header,
    /// Authentication failed (wrong key, metadata or tampering).
    Open,
    /// The AEAD refused to encrypt (plaintext beyond the GCM limit).
    Seal,
}

impl std::fmt::Display for SframeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Header => "malformed SFrame header",
            Self::Open => "SFrame authentication failed",
            Self::Seal => "SFrame encryption failed",
        })
    }
}

impl std::error::Error for SframeError {}

/// The `sframe_key` and `sframe_salt` for one KID (§4.4.2).
pub struct SframeKey {
    key: Zeroizing<[u8; NK]>,
    salt: [u8; NN],
}

/// Derive the key and salt for `kid` from `base_key` (§4.4.2, HKDF-SHA512
/// for suite 0x0005).
#[must_use]
pub fn derive(base_key: &[u8], kid: Kid) -> SframeKey {
    let hk = Hkdf::<Sha512>::new(Some(&[]), base_key);
    let mut key = Zeroizing::new([0u8; NK]);
    let mut salt = [0u8; NN];
    // Expand cannot fail: 32 and 12 bytes are far below 255 * HashLen.
    let _ = hk.expand(&label(b"SFrame 1.0 Secret key ", kid), key.as_mut());
    let _ = hk.expand(&label(b"SFrame 1.0 Secret salt ", kid), &mut salt);
    SframeKey { key, salt }
}

fn label(prefix: &[u8], kid: Kid) -> Vec<u8> {
    let mut out = Vec::with_capacity(prefix.len() + 10);
    out.extend_from_slice(prefix);
    out.extend_from_slice(&kid.0.to_be_bytes());
    out.extend_from_slice(&SUITE_AES_256_GCM_SHA512_128.to_be_bytes());
    out
}

/// Append the header for `(kid, ctr)` to `out` (§4.3).
pub fn encode_header(kid: Kid, ctr: u64, out: &mut Vec<u8>) {
    let (k_bits, k_bytes) = compact(kid.0);
    let (c_bits, c_bytes) = compact(ctr);
    out.push((k_bits << 4) | c_bits);
    out.extend_from_slice(k_bytes.as_slice());
    out.extend_from_slice(c_bytes.as_slice());
}

/// A header field: the 4 config bits (flag + value or length - 1) and the
/// big-endian bytes that follow (empty for values 0-7).
fn compact(value: u64) -> (u8, MinimalBytes) {
    match u8::try_from(value) {
        Ok(small) if small < 8 => (small, MinimalBytes::default()),
        _ => {
            let bytes = MinimalBytes::of(value);
            // 2..=8 bytes here, so `len - 1` is 1..=7 and fits 3 bits.
            let len_minus_one = u8::try_from(bytes.len - 1).unwrap_or(7);
            (0b1000 | len_minus_one, bytes)
        }
    }
}

/// `value` as its minimal big-endian bytes.
#[derive(Default)]
struct MinimalBytes {
    buf: [u8; 8],
    len: usize,
}

impl MinimalBytes {
    fn of(value: u64) -> Self {
        let leading = value.leading_zeros() as usize / 8;
        Self {
            buf: value.to_be_bytes(),
            len: 8 - leading,
        }
    }

    fn as_slice(&self) -> &[u8] {
        &self.buf[8 - self.len..]
    }
}

/// Parse a frame's header: `(kid, ctr, header_len)` (§4.3).
pub fn parse_header(frame: &[u8]) -> Result<(Kid, u64, usize), SframeError> {
    let config = *frame.first().ok_or(SframeError::Header)?;
    let mut at = 1;
    let mut field = |bits: u8| -> Result<u64, SframeError> {
        if bits & 0b1000 == 0 {
            return Ok(u64::from(bits & 0b111));
        }
        let len = usize::from(bits & 0b111) + 1;
        let bytes = frame.get(at..at + len).ok_or(SframeError::Header)?;
        at += len;
        Ok(bytes.iter().fold(0u64, |acc, b| (acc << 8) | u64::from(*b)))
    };
    let kid = field(config >> 4)?;
    let ctr = field(config & 0x0f)?;
    Ok((Kid(kid), ctr, at))
}

fn nonce(salt: &[u8; NN], ctr: u64) -> [u8; NN] {
    let mut nonce = *salt;
    for (n, c) in nonce[NN - 8..].iter_mut().zip(ctr.to_be_bytes()) {
        *n ^= c;
    }
    nonce
}

/// Encrypt `plaintext` under `key` as `(kid, ctr)`, authenticating
/// `metadata` (§4.4.3). Returns `header ‖ ciphertext`. The caller must
/// never reuse `(kid, ctr)` for a key (§9.1); [`SframeSender`] hands out
/// unique counters.
pub fn seal(
    key: &SframeKey,
    kid: Kid,
    ctr: u64,
    metadata: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, SframeError> {
    let mut frame = Vec::with_capacity(MAX_HEADER_LEN + plaintext.len() + 16);
    encode_header(kid, ctr, &mut frame);
    let mut aad = Vec::with_capacity(frame.len() + metadata.len());
    aad.extend_from_slice(&frame);
    aad.extend_from_slice(metadata);
    let ciphertext = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key.key.as_ref()))
        .encrypt(
            Nonce::from_slice(&nonce(&key.salt, ctr)),
            Payload {
                msg: plaintext,
                aad: &aad,
            },
        )
        .map_err(|_| SframeError::Seal)?;
    frame.extend_from_slice(&ciphertext);
    Ok(frame)
}

/// Decrypt a frame whose key was selected from its header (§4.4.4).
pub fn open(key: &SframeKey, frame: &[u8], metadata: &[u8]) -> Result<Vec<u8>, SframeError> {
    let (_, ctr, header_len) = parse_header(frame)?;
    let (header, ciphertext) = frame.split_at(header_len);
    let mut aad = Vec::with_capacity(header.len() + metadata.len());
    aad.extend_from_slice(header);
    aad.extend_from_slice(metadata);
    Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key.key.as_ref()))
        .decrypt(
            Nonce::from_slice(&nonce(&key.salt, ctr)),
            Payload {
                msg: ciphertext,
                aad: &aad,
            },
        )
        .map_err(|_| SframeError::Open)
}

// ── Rekindle sender keying (§5.1) ────────────────────────────────────

/// A media KID: the sender's 56-bit session tag above the low 8 bits of
/// the key generation.
#[must_use]
pub fn media_kid(tag: u64, generation: u64) -> Kid {
    Kid(((tag & TAG_MASK) << GENERATION_BITS) | (generation & 0xff))
}

/// The sender tag a media KID carries.
#[must_use]
pub fn kid_tag(kid: Kid) -> u64 {
    kid.0 >> GENERATION_BITS
}

/// The low 8 bits of the key generation a media KID carries.
#[must_use]
pub fn kid_generation_low(kid: Kid) -> u8 {
    kid.0.to_be_bytes()[7]
}

/// Whether `generation` is the one a KID's low generation bits name.
#[must_use]
pub fn kid_names_generation(kid: Kid, generation: u64) -> bool {
    u64::from(kid_generation_low(kid)) == generation & 0xff
}

/// A sender's base key: `HKDF-SHA512(salt = "", ikm = scope_secret,
/// info = VOICE_SENDER_KEY ‖ sender_key ‖ tag BE)`. Distinct for every
/// sender and session sharing the scope secret.
#[must_use]
pub fn sender_base_key(
    scope_secret: &[u8; 32],
    sender_key: &[u8],
    tag: u64,
) -> Zeroizing<[u8; NK]> {
    let hk = Hkdf::<Sha512>::new(Some(&[]), scope_secret);
    let mut info = Vec::with_capacity(VOICE_SENDER_KEY.len() + sender_key.len() + 8);
    info.extend_from_slice(VOICE_SENDER_KEY.as_bytes());
    info.extend_from_slice(sender_key);
    info.extend_from_slice(&(tag & TAG_MASK).to_be_bytes());
    let mut out = Zeroizing::new([0u8; NK]);
    let _ = hk.expand(&info, out.as_mut());
    out
}

/// The SFrame key a sender uses for `kid`, from the scope secret.
#[must_use]
pub fn media_key(scope_secret: &[u8; 32], sender_key: &[u8], kid: Kid) -> SframeKey {
    derive(
        sender_base_key(scope_secret, sender_key, kid_tag(kid)).as_ref(),
        kid,
    )
}

/// Our sending state for one scope and key generation: the session tag
/// and the next CTR. Shared by every transport the session builds, so a
/// rebuilt transport continues the counter (§9.1).
#[derive(Debug)]
pub struct SframeSender {
    tag: u64,
    next_ctr: AtomicU64,
}

impl SframeSender {
    /// A sender with a fresh random tag, counting from 0.
    #[must_use]
    pub fn fresh() -> Self {
        use rand::RngCore as _;
        Self {
            tag: rand::rngs::OsRng.next_u64() & TAG_MASK,
            next_ctr: AtomicU64::new(0),
        }
    }

    /// The KID this sender uses for `generation`.
    #[must_use]
    pub fn kid(&self, generation: u64) -> Kid {
        media_kid(self.tag, generation)
    }

    /// Reserve the next CTR. Never returns the same value twice.
    pub fn next_ctr(&self) -> u64 {
        self.next_ctr.fetch_add(1, Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests;
