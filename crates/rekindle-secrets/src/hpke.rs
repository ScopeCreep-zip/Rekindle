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
mod tests {
    use super::*;

    fn h(s: &str) -> Vec<u8> {
        hex::decode(s).unwrap()
    }

    fn h32(s: &str) -> [u8; 32] {
        h(s).try_into().unwrap()
    }

    /// RFC 9180 Appendix A.2's shared inputs.
    const INFO: &str = "Ode on a Grecian Urn";
    const AAD: &[u8] = b"Count-0";
    const PT: &[u8] = b"Beauty is truth, truth beauty";

    /// RFC 9180 A.2.1 (Base, X25519 / HKDF-SHA256 / ChaCha20-Poly1305),
    /// sequence number 0.
    #[test]
    fn rfc9180_base_vector_opens() {
        let sk_r = h32("8057991eef8f1f1af18f4a9491d16a1ce333f695d4db8e38da75975c4478e0fb");
        assert_eq!(
            public_key(&sk_r),
            h32("4310ee97d88cc1f088a5576c77ab0cf5c3ac797f3d95139c6c84b5429c59662a")
        );
        let pt = open_base(
            &sk_r,
            INFO,
            AAD,
            &h("1afa08d3dec047a643885163f1180476fa7ddb54c6a8029ea33f95796bf2ac4a"),
            &h("1c5250d8034ec2b784ba2cfd69dbdb8af406cfe3ff938e131f0def8c8b60b4db21993c62ce81883d2dd1b51a28"),
        )
        .unwrap();
        assert_eq!(pt.as_slice(), PT);
    }

    /// RFC 9180 A.2.3 (Auth, same suite), sequence number 0.
    #[test]
    fn rfc9180_auth_vector_opens() {
        let sk_r = h32("3ca22a6d1cda1bb9480949ec5329d3bf0b080ca4c45879c95eddb55c70b80b82");
        let sk_s = h32("2def0cb58ffcf83d1062dd085c8aceca7f4c0c3fd05912d847b61f3e54121f05");
        let pk_s = h32("f0f4f9e96c54aeed3f323de8534fffd7e0577e4ce269896716bcb95643c8712b");
        assert_eq!(public_key(&sk_s), pk_s);
        let enc = h("f7674cc8cd7baa5872d1f33dbaffe3314239f6197ddf5ded1746760bfc847e0e");
        let ct = h("ab1a13c9d4f01a87ec3440dbd756e2677bd2ecf9df0ce7ed73869b98e00c09be111cb9fdf077347aeb88e61bdf");
        let pt = open_auth(&sk_r, &pk_s, INFO, AAD, &enc, &ct).unwrap();
        assert_eq!(pt.as_slice(), PT);
        // Auth binds the sender: any other claimed sender fails.
        assert!(open_auth(&sk_r, &public_key(&[9; 32]), INFO, AAD, &enc, &ct).is_err());
        // And it is not a Base-mode message.
        assert!(open_base(&sk_r, INFO, AAD, &enc, &ct).is_err());
    }

    #[test]
    fn base_round_trip_binds_key_info_and_aad() {
        let (bob, carol) = ([2u8; 32], [3u8; 32]);
        let sealed = seal_base(&public_key(&bob), INFO, AAD, PT).unwrap();
        assert_eq!(sealed.ciphertext.len(), PT.len() + TAG_LEN);
        let open = |sk: &[u8; 32], info: &str, aad: &[u8]| {
            open_base(sk, info, aad, &sealed.enc, &sealed.ciphertext)
        };
        assert_eq!(open(&bob, INFO, AAD).unwrap().as_slice(), PT);
        assert!(open(&carol, INFO, AAD).is_err());
        assert!(open(&bob, "Ode to a Nightingale", AAD).is_err());
        assert!(open(&bob, INFO, b"Count-1").is_err());
    }

    #[test]
    fn auth_round_trip_binds_the_sender() {
        let (alice, bob, mallory) = ([1u8; 32], [2u8; 32], [4u8; 32]);
        let sealed = seal_auth(&alice, &public_key(&bob), INFO, AAD, PT).unwrap();
        let open = |sender: &[u8; 32]| {
            open_auth(
                &bob,
                &public_key(sender),
                INFO,
                AAD,
                &sealed.enc,
                &sealed.ciphertext,
            )
        };
        assert_eq!(open(&alice).unwrap().as_slice(), PT);
        assert!(open(&mallory).is_err());
    }

    #[test]
    fn a_malformed_or_tampered_message_fails() {
        let bob = [2u8; 32];
        let sealed = seal_base(&public_key(&bob), INFO, AAD, PT).unwrap();
        assert!(open_base(&bob, INFO, AAD, &sealed.enc[..31], &sealed.ciphertext).is_err());
        let mut ct = sealed.ciphertext.clone();
        ct[0] ^= 1;
        assert!(open_base(&bob, INFO, AAD, &sealed.enc, &ct).is_err());
    }
}
