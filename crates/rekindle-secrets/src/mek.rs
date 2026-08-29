//! MEK (Media Encryption Key) wrapping for peer-to-peer distribution.
//!
//! The sender (deterministic rotator) wraps the MEK for each recipient
//! using their pseudonym public key. No coordinator involved — any peer
//! can wrap/unwrap.
//!
//! Two wire formats, distinguished by the leading byte:
//!
//! - **v1 (legacy, no version byte)**: hand-rolled X25519 static-static
//!   ECDH → HKDF-SHA256 (`rekindle-mek-wrap-v1`) → AES-256-GCM.
//!   `[12-byte nonce || ciphertext + 16-byte tag]` (68 bytes for the
//!   40-byte MEK wire input).
//! - **v2 (`0x02` prefix)**: RFC 9180 HPKE, DHKEM-X25519 + HKDF-SHA256 +
//!   ChaCha20Poly1305, **Auth mode** — the static-static shape of v1,
//!   standardized. Auth (not Base) is deliberate: v1's ECDH implicitly
//!   authenticated the sender, and Base mode would silently drop that
//!   property. Info label `rekindle-mek/1` (deliberately NOT Veilid's
//!   `veilid-hpke/1` — MEK wrapping is Rekindle↔Rekindle only, and our
//!   wire format must not be coupled to Veilid's domain separation).
//!   `[0x02 || enc(32) || ciphertext + 16-byte tag]`.
//!
//! **Rollout**: [`unwrap_mek`] reads BOTH formats (see its docs for the
//! version-byte/nonce-collision handling). [`wrap_mek`] still emits v1;
//! senders flip to [`hpke_wrap_mek`] once every deployed reader carries
//! this dual-read version. The Ed25519→X25519 bridge is byte-identical
//! to Veilid's VLD0 derivation (verified in
//! `docs/contributor/veilid-provided-vs-home-rolled.md` §1.1), so the
//! same pseudonym keys serve both formats — no re-keying.

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use ed25519_dalek::{SigningKey, VerifyingKey};
use hkdf::Hkdf;
use rand::RngCore;
use rekindle_types::error::CryptoError;
use sha2::Sha256;
use x25519_dalek::PublicKey as X25519PublicKey;
use zeroize::Zeroizing;

use crate::derive::pseudonym_to_x25519;

/// HKDF info label for MEK wrapping key derivation.
const HKDF_INFO: &[u8] = b"rekindle-mek-wrap-v1";

/// Derive an AES-256-GCM wrapping key from an X25519 shared secret.
/// The return type wraps the bytes in `Zeroizing` so the wrapping key
/// is scrubbed from memory when it goes out of scope, even on the
/// happy path. Audit finding (P7-W26).
fn derive_wrapping_key(shared_secret: &x25519_dalek::SharedSecret) -> Zeroizing<[u8; 32]> {
    let hkdf = Hkdf::<Sha256>::new(None, shared_secret.as_bytes());
    let mut key = Zeroizing::new([0u8; 32]);
    hkdf.expand(HKDF_INFO, key.as_mut())
        .expect("32-byte output is valid for HKDF-SHA256");
    key
}

/// Wrap (encrypt) MEK wire bytes for a specific recipient.
///
/// - `sender_signing_key`: The wrapping peer's Ed25519 pseudonym key.
/// - `recipient_ed25519_public`: The target member's Ed25519 public key bytes.
/// - `mek_wire_bytes`: The 40-byte MEK wire format `[generation LE || key]`.
///
/// Returns: `[12-byte nonce || ciphertext + 16-byte tag]` (68 bytes).
pub fn wrap_mek(
    sender_signing_key: &SigningKey,
    recipient_ed25519_public: &[u8; 32],
    mek_wire_bytes: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let sender_x25519 = pseudonym_to_x25519(sender_signing_key);

    let recipient_verifying = VerifyingKey::from_bytes(recipient_ed25519_public)
        .map_err(|e| CryptoError::InvalidKey(format!("invalid recipient Ed25519 key: {e}")))?;
    let recipient_x25519 = X25519PublicKey::from(recipient_verifying.to_montgomery().to_bytes());

    let shared_secret = sender_x25519.diffie_hellman(&recipient_x25519);
    let wrapping_key = derive_wrapping_key(&shared_secret);

    let cipher = Aes256Gcm::new_from_slice(&wrapping_key[..])
        .map_err(|e| CryptoError::Encryption(format!("AES-GCM init: {e}")))?;

    let mut nonce_bytes = [0u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, mek_wire_bytes)
        .map_err(|e| CryptoError::Encryption(format!("AES-GCM encrypt: {e}")))?;

    let mut output = Vec::with_capacity(12 + ciphertext.len());
    output.extend_from_slice(&nonce_bytes);
    output.extend_from_slice(&ciphertext);
    Ok(output)
}

/// Version byte prefixing HPKE-wrapped (v2) MEK blobs.
pub const HPKE_MEK_VERSION: u8 = 0x02;

/// HPKE info label for MEK wrapping (v2). Deliberately Rekindle-owned —
/// never Veilid's `veilid-hpke/1` (see module docs).
const HPKE_MEK_INFO: &[u8] = b"rekindle-mek/1";

type HpkeKem = hpke::kem::X25519HkdfSha256;
type HpkeKdf = hpke::kdf::HkdfSha256;
type HpkeAead = hpke::aead::ChaCha20Poly1305;

/// Convert our Ed25519-derived X25519 keys into the hpke crate's types.
fn hpke_keys(
    our_signing_key: &SigningKey,
    their_ed25519_public: &[u8; 32],
) -> Result<
    (
        <HpkeKem as hpke::Kem>::PrivateKey,
        <HpkeKem as hpke::Kem>::PublicKey,
        <HpkeKem as hpke::Kem>::PublicKey,
    ),
    CryptoError,
> {
    use hpke::Deserializable;
    let our_x25519 = pseudonym_to_x25519(our_signing_key);
    let our_sk = <HpkeKem as hpke::Kem>::PrivateKey::from_bytes(&our_x25519.to_bytes())
        .map_err(|e| CryptoError::InvalidKey(format!("own X25519 key: {e}")))?;
    let our_pk = <HpkeKem as hpke::Kem>::sk_to_pk(&our_sk);

    let their_verifying = VerifyingKey::from_bytes(their_ed25519_public)
        .map_err(|e| CryptoError::InvalidKey(format!("invalid peer Ed25519 key: {e}")))?;
    let their_pk =
        <HpkeKem as hpke::Kem>::PublicKey::from_bytes(&their_verifying.to_montgomery().to_bytes())
            .map_err(|e| CryptoError::InvalidKey(format!("peer X25519 key: {e}")))?;

    Ok((our_sk, our_pk, their_pk))
}

/// Wrap MEK wire bytes for a recipient via RFC 9180 HPKE (v2 format).
///
/// Same parameters and semantics as [`wrap_mek`]; output is
/// `[0x02 || enc(32) || ciphertext+tag]`. Senders switch to this once
/// every deployed reader understands the dual-read [`unwrap_mek`].
pub fn hpke_wrap_mek(
    sender_signing_key: &SigningKey,
    recipient_ed25519_public: &[u8; 32],
    mek_wire_bytes: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    use hpke::Serializable;
    let (our_sk, our_pk, their_pk) = hpke_keys(sender_signing_key, recipient_ed25519_public)?;

    let (enc, ciphertext) = hpke::single_shot_seal::<HpkeAead, HpkeKdf, HpkeKem>(
        &hpke::OpModeS::Auth((our_sk, our_pk)),
        &their_pk,
        HPKE_MEK_INFO,
        mek_wire_bytes,
        b"",
    )
    .map_err(|e| CryptoError::Encryption(format!("HPKE seal: {e}")))?;

    let enc_bytes = enc.to_bytes();
    let mut output = Vec::with_capacity(1 + enc_bytes.len() + ciphertext.len());
    output.push(HPKE_MEK_VERSION);
    output.extend_from_slice(&enc_bytes);
    output.extend_from_slice(&ciphertext);
    Ok(output)
}

/// Open an HPKE-wrapped (v2) MEK blob. Expects the `0x02` version byte.
pub fn hpke_open_mek(
    recipient_signing_key: &SigningKey,
    sender_ed25519_public: &[u8; 32],
    wrapped_mek: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    use hpke::Deserializable;
    // 1 version + 32 enc + 16 tag minimum
    if wrapped_mek.len() < 49 || wrapped_mek[0] != HPKE_MEK_VERSION {
        return Err(CryptoError::Decryption("not an HPKE-wrapped MEK".into()));
    }
    let (our_sk, _our_pk, their_pk) = hpke_keys(recipient_signing_key, sender_ed25519_public)?;

    let enc = <HpkeKem as hpke::Kem>::EncappedKey::from_bytes(&wrapped_mek[1..33])
        .map_err(|e| CryptoError::Decryption(format!("HPKE encapped key: {e}")))?;

    hpke::single_shot_open::<HpkeAead, HpkeKdf, HpkeKem>(
        &hpke::OpModeR::Auth(their_pk),
        &our_sk,
        &enc,
        HPKE_MEK_INFO,
        &wrapped_mek[33..],
        b"",
    )
    .map_err(|e| CryptoError::Decryption(format!("HPKE open: {e}")))
}

/// Unwrap (decrypt) MEK wire bytes received from a peer — reads BOTH
/// wire formats.
///
/// A `0x02` leading byte selects the HPKE (v2) path first. Because v1
/// blobs start with a random nonce, 1-in-256 of them ALSO lead with
/// `0x02`; the v2 attempt then fails authentication (AEAD — a misparse
/// cannot false-succeed) and the blob falls through to the v1 path, so
/// legacy blobs always decrypt regardless of their nonce.
///
/// - `recipient_signing_key`: Our Ed25519 pseudonym signing key.
/// - `sender_ed25519_public`: The wrapping peer's Ed25519 public key bytes.
///
/// Returns: The decrypted MEK wire bytes (40 bytes).
pub fn unwrap_mek(
    recipient_signing_key: &SigningKey,
    sender_ed25519_public: &[u8; 32],
    wrapped_mek: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    if wrapped_mek.first() == Some(&HPKE_MEK_VERSION) {
        if let Ok(plaintext) =
            hpke_open_mek(recipient_signing_key, sender_ed25519_public, wrapped_mek)
        {
            return Ok(plaintext);
        }
        // Fall through: presumably a v1 blob whose nonce starts 0x02.
    }
    unwrap_mek_v1(recipient_signing_key, sender_ed25519_public, wrapped_mek)
}

/// The legacy (v1) unwrap path: X25519 ECDH → HKDF → AES-256-GCM over
/// `[12-byte nonce || ciphertext + tag]`.
fn unwrap_mek_v1(
    recipient_signing_key: &SigningKey,
    sender_ed25519_public: &[u8; 32],
    wrapped_mek: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    if wrapped_mek.len() < 12 {
        return Err(CryptoError::Decryption("wrapped MEK too short".into()));
    }

    let recipient_x25519 = pseudonym_to_x25519(recipient_signing_key);

    let sender_verifying = VerifyingKey::from_bytes(sender_ed25519_public)
        .map_err(|e| CryptoError::InvalidKey(format!("invalid sender Ed25519 key: {e}")))?;
    let sender_x25519 = X25519PublicKey::from(sender_verifying.to_montgomery().to_bytes());

    let shared_secret = recipient_x25519.diffie_hellman(&sender_x25519);
    let wrapping_key = derive_wrapping_key(&shared_secret);

    let cipher = Aes256Gcm::new_from_slice(&wrapping_key[..])
        .map_err(|e| CryptoError::Decryption(format!("AES-GCM init: {e}")))?;

    let nonce = Nonce::from_slice(&wrapped_mek[..12]);
    cipher
        .decrypt(nonce, &wrapped_mek[12..])
        .map_err(|e| CryptoError::Decryption(format!("AES-GCM decrypt: {e}")))?
        .pipe(Ok)
}

/// Extension trait for method chaining.
trait Pipe: Sized {
    fn pipe<R>(self, f: impl FnOnce(Self) -> R) -> R {
        f(self)
    }
}
impl<T> Pipe for T {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::derive::derive_community_pseudonym;
    use crate::keys::MediaEncryptionKey;

    #[test]
    fn wrap_unwrap_roundtrip() {
        let sender_key = derive_community_pseudonym(&[1u8; 32], "test");
        let recipient_key = derive_community_pseudonym(&[2u8; 32], "test");
        let mek = MediaEncryptionKey::generate(42);
        let wire = mek.to_wire_bytes();

        let wrapped = wrap_mek(
            &sender_key,
            &recipient_key.verifying_key().to_bytes(),
            &wire,
        )
        .unwrap();
        assert_eq!(wrapped.len(), 68);

        let unwrapped = unwrap_mek(
            &recipient_key,
            &sender_key.verifying_key().to_bytes(),
            &wrapped,
        )
        .unwrap();
        assert_eq!(unwrapped, wire);

        let restored = MediaEncryptionKey::from_wire_bytes(&unwrapped).unwrap();
        assert_eq!(restored.generation(), 42);
        assert_eq!(restored.as_bytes(), mek.as_bytes());
    }

    #[test]
    fn wrong_recipient_fails() {
        let sender = derive_community_pseudonym(&[1u8; 32], "c");
        let recipient = derive_community_pseudonym(&[2u8; 32], "c");
        let wrong = derive_community_pseudonym(&[3u8; 32], "c");

        let mek = MediaEncryptionKey::generate(1);
        let wrapped = wrap_mek(
            &sender,
            &recipient.verifying_key().to_bytes(),
            &mek.to_wire_bytes(),
        )
        .unwrap();

        assert!(unwrap_mek(&wrong, &sender.verifying_key().to_bytes(), &wrapped,).is_err());
    }

    #[test]
    fn wrong_sender_key_fails() {
        let sender = derive_community_pseudonym(&[1u8; 32], "c");
        let fake_sender = derive_community_pseudonym(&[99u8; 32], "c");
        let recipient = derive_community_pseudonym(&[2u8; 32], "c");

        let mek = MediaEncryptionKey::generate(1);
        let wrapped = wrap_mek(
            &sender,
            &recipient.verifying_key().to_bytes(),
            &mek.to_wire_bytes(),
        )
        .unwrap();

        assert!(unwrap_mek(
            &recipient,
            &fake_sender.verifying_key().to_bytes(),
            &wrapped,
        )
        .is_err());
    }

    #[test]
    fn wrapped_too_short() {
        let sender = derive_community_pseudonym(&[1u8; 32], "c");
        let recipient = derive_community_pseudonym(&[2u8; 32], "c");
        assert!(unwrap_mek(&recipient, &sender.verifying_key().to_bytes(), &[0u8; 11],).is_err());
    }

    #[test]
    fn hpke_wrap_open_roundtrip() {
        let sender = derive_community_pseudonym(&[1u8; 32], "c");
        let recipient = derive_community_pseudonym(&[2u8; 32], "c");
        let mek = MediaEncryptionKey::generate(7);
        let wire = mek.to_wire_bytes();

        let wrapped = hpke_wrap_mek(&sender, &recipient.verifying_key().to_bytes(), &wire).unwrap();
        assert_eq!(wrapped[0], HPKE_MEK_VERSION);
        // 1 version + 32 enc + 40 plaintext + 16 tag = 89
        assert_eq!(wrapped.len(), 89);

        let opened =
            hpke_open_mek(&recipient, &sender.verifying_key().to_bytes(), &wrapped).unwrap();
        assert_eq!(opened, wire);
    }

    #[test]
    fn hpke_blob_opens_through_dual_read_unwrap() {
        let sender = derive_community_pseudonym(&[1u8; 32], "c");
        let recipient = derive_community_pseudonym(&[2u8; 32], "c");
        let mek = MediaEncryptionKey::generate(9);
        let wire = mek.to_wire_bytes();

        let wrapped = hpke_wrap_mek(&sender, &recipient.verifying_key().to_bytes(), &wire).unwrap();
        let opened = unwrap_mek(&recipient, &sender.verifying_key().to_bytes(), &wrapped).unwrap();
        assert_eq!(opened, wire);
    }

    #[test]
    fn hpke_auth_mode_rejects_wrong_sender() {
        // Auth mode preserves v1's sender authentication: opening with
        // the wrong claimed sender must fail.
        let sender = derive_community_pseudonym(&[1u8; 32], "c");
        let fake_sender = derive_community_pseudonym(&[9u8; 32], "c");
        let recipient = derive_community_pseudonym(&[2u8; 32], "c");
        let mek = MediaEncryptionKey::generate(1);

        let wrapped = hpke_wrap_mek(
            &sender,
            &recipient.verifying_key().to_bytes(),
            &mek.to_wire_bytes(),
        )
        .unwrap();
        assert!(hpke_open_mek(
            &recipient,
            &fake_sender.verifying_key().to_bytes(),
            &wrapped
        )
        .is_err());
    }

    #[test]
    fn hpke_tampered_ciphertext_rejected() {
        let sender = derive_community_pseudonym(&[1u8; 32], "c");
        let recipient = derive_community_pseudonym(&[2u8; 32], "c");
        let mek = MediaEncryptionKey::generate(1);

        let mut wrapped = hpke_wrap_mek(
            &sender,
            &recipient.verifying_key().to_bytes(),
            &mek.to_wire_bytes(),
        )
        .unwrap();
        let last = wrapped.len() - 1;
        wrapped[last] ^= 0xFF;
        assert!(hpke_open_mek(&recipient, &sender.verifying_key().to_bytes(), &wrapped).is_err());
    }

    #[test]
    fn v1_blob_with_version_colliding_nonce_still_opens() {
        // A legacy v1 blob whose random nonce happens to start with the
        // v2 version byte must fall through the failed HPKE attempt and
        // decrypt via the v1 path. Expected ~1 collision per 256 wraps.
        let sender = derive_community_pseudonym(&[1u8; 32], "c");
        let recipient = derive_community_pseudonym(&[2u8; 32], "c");
        let mek = MediaEncryptionKey::generate(3);
        let wire = mek.to_wire_bytes();

        for _ in 0..10_000 {
            let wrapped = wrap_mek(&sender, &recipient.verifying_key().to_bytes(), &wire).unwrap();
            if wrapped[0] == HPKE_MEK_VERSION {
                let opened =
                    unwrap_mek(&recipient, &sender.verifying_key().to_bytes(), &wrapped).unwrap();
                assert_eq!(opened, wire);
                return;
            }
        }
        panic!("no 0x02-leading v1 nonce in 10k wraps — statistically broken RNG");
    }

    #[test]
    fn cross_community_fails() {
        let sender_a = derive_community_pseudonym(&[1u8; 32], "community_a");
        let recipient_a = derive_community_pseudonym(&[2u8; 32], "community_a");
        let recipient_b = derive_community_pseudonym(&[2u8; 32], "community_b");
        let sender_b = derive_community_pseudonym(&[1u8; 32], "community_b");

        let mek = MediaEncryptionKey::generate(1);
        let wrapped = wrap_mek(
            &sender_a,
            &recipient_a.verifying_key().to_bytes(),
            &mek.to_wire_bytes(),
        )
        .unwrap();

        assert!(unwrap_mek(&recipient_b, &sender_b.verifying_key().to_bytes(), &wrapped,).is_err());
    }
}
