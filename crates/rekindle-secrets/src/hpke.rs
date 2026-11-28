//! The one RFC 9180 HPKE wrapper (plan E2.3).
//!
//! One suite: DHKEM(X25519, HKDF-SHA256), HKDF-SHA256, ChaCha20-Poly1305
//! (RFC 9180 §7: KEM `0x0020`, KDF `0x0001`, AEAD `0x0003`), single-shot,
//! in two modes:
//!
//! - **Base** ([`seal_base`], [`open_base`]): only the recipient's key is
//!   bound. It does not authenticate the sender at all (§9.8), so every
//!   Base-mode user signs `(enc, ct)` with the sender's key as §9.1.1
//!   recommends: the call-media keys (`media_sender_key::session`) and the
//!   DM pre-session frames, sealed to the recipient's identity X25519.
//! - **Auth** ([`seal_auth`], [`open_auth`]): the sender's static X25519
//!   key is bound too. It authenticates only that key and is open to
//!   key-compromise impersonation (§9.1.1); its one user is the v2 MEK wrap
//!   (`mek.rs`).
//!
//! The `info` label is always a `rekindle_types::domains` constant, so no
//! two purposes share a key schedule. Keys are raw 32-byte X25519 scalars
//! and points; the callers convert from Ed25519 where a key starts there.
//! [`Sealed`] keeps `enc` and the ciphertext apart so a signature can cover
//! both and each wire format lays them out as it already does.

use ::hpke::{Deserializable, Kem as _, OpModeR, OpModeS, Serializable};
use rekindle_types::error::CryptoError;
use zeroize::Zeroizing;

type Kem = ::hpke::kem::X25519HkdfSha256;
type Kdf = ::hpke::kdf::HkdfSha256;
type Aead = ::hpke::aead::ChaCha20Poly1305;

type PrivateKey = <Kem as ::hpke::Kem>::PrivateKey;
type PublicKey = <Kem as ::hpke::Kem>::PublicKey;
type EncappedKey = <Kem as ::hpke::Kem>::EncappedKey;

/// Length of the encapsulated key `enc` (`Nenc`, RFC 9180 §7.1).
pub const ENC_LEN: usize = 32;
/// Length of the ChaCha20-Poly1305 tag a ciphertext adds (`Nt`, §7.3).
pub const TAG_LEN: usize = 16;

/// One sealed message: the encapsulated key and the ciphertext with its tag.
pub struct Sealed {
    /// The ephemeral public key the recipient decapsulates.
    pub enc: [u8; ENC_LEN],
    /// The plaintext's ciphertext, `TAG_LEN` bytes longer than it.
    pub ciphertext: Vec<u8>,
}

/// The X25519 public key of `secret` (RFC 9180 §7.1.3 `SerializePublicKey`
/// of `sk_to_pk`).
#[must_use]
pub fn public_key(secret: &[u8; 32]) -> [u8; 32] {
    let pk = Kem::sk_to_pk(&private_key(secret));
    let mut out = [0u8; 32];
    out.copy_from_slice(&pk.to_bytes());
    out
}

/// Seal `plaintext` to `recipient` in Base mode.
///
/// # Errors
/// An HPKE failure (a recipient key that yields an all-zero shared secret).
pub fn seal_base(
    recipient: &[u8; 32],
    info: &str,
    aad: &[u8],
    plaintext: &[u8],
) -> Result<Sealed, CryptoError> {
    seal(&OpModeS::Base, recipient, info, aad, plaintext)
}

/// Open a Base-mode message sealed to the public half of `recipient_secret`.
///
/// # Errors
/// A malformed `enc`, or a message not sealed to this key under this `info`
/// and `aad`.
pub fn open_base(
    recipient_secret: &[u8; 32],
    info: &str,
    aad: &[u8],
    enc: &[u8],
    ciphertext: &[u8],
) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
    open(&OpModeR::Base, recipient_secret, info, aad, enc, ciphertext)
}

/// Seal `plaintext` to `recipient` in Auth mode, binding `sender_secret`.
///
/// # Errors
/// An HPKE failure (a key that yields an all-zero shared secret).
pub fn seal_auth(
    sender_secret: &[u8; 32],
    recipient: &[u8; 32],
    info: &str,
    aad: &[u8],
    plaintext: &[u8],
) -> Result<Sealed, CryptoError> {
    let sk = private_key(sender_secret);
    let pk = Kem::sk_to_pk(&sk);
    seal(&OpModeS::Auth((sk, pk)), recipient, info, aad, plaintext)
}

/// Open an Auth-mode message from `sender` sealed to the public half of
/// `recipient_secret`.
///
/// # Errors
/// A malformed key or `enc`, or a message not sealed by `sender` to this key
/// under this `info` and `aad`.
pub fn open_auth(
    recipient_secret: &[u8; 32],
    sender: &[u8; 32],
    info: &str,
    aad: &[u8],
    enc: &[u8],
    ciphertext: &[u8],
) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
    let mode = OpModeR::Auth(public(sender, "sender")?);
    open(&mode, recipient_secret, info, aad, enc, ciphertext)
}

fn seal(
    mode: &OpModeS<Kem>,
    recipient: &[u8; 32],
    info: &str,
    aad: &[u8],
    plaintext: &[u8],
) -> Result<Sealed, CryptoError> {
    let pk = public(recipient, "recipient")?;
    let (enc, ciphertext) =
        ::hpke::single_shot_seal::<Aead, Kdf, Kem>(mode, &pk, info.as_bytes(), plaintext, aad)
            .map_err(|e| CryptoError::Encryption(format!("HPKE seal: {e}")))?;
    let mut enc_bytes = [0u8; ENC_LEN];
    enc_bytes.copy_from_slice(&enc.to_bytes());
    Ok(Sealed {
        enc: enc_bytes,
        ciphertext,
    })
}

fn open(
    mode: &OpModeR<Kem>,
    recipient_secret: &[u8; 32],
    info: &str,
    aad: &[u8],
    enc: &[u8],
    ciphertext: &[u8],
) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
    let enc = EncappedKey::from_bytes(enc)
        .map_err(|e| CryptoError::Decryption(format!("HPKE encapped key: {e}")))?;
    ::hpke::single_shot_open::<Aead, Kdf, Kem>(
        mode,
        &private_key(recipient_secret),
        &enc,
        info.as_bytes(),
        ciphertext,
        aad,
    )
    .map(Zeroizing::new)
    .map_err(|e| CryptoError::Decryption(format!("HPKE open: {e}")))
}

/// Any 32 bytes are an X25519 private key (RFC 7748 §5 clamps them).
fn private_key(secret: &[u8; 32]) -> PrivateKey {
    PrivateKey::from_bytes(secret).expect("any 32 bytes are an X25519 private key")
}

fn public(key: &[u8; 32], role: &str) -> Result<PublicKey, CryptoError> {
    PublicKey::from_bytes(key)
        .map_err(|e| CryptoError::InvalidKey(format!("{role} X25519 key: {e}")))
}

#[cfg(test)]
#[path = "hpke_tests.rs"]
mod tests;
