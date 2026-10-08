//! Call-media sender keys (plan C7.20; RFC 9605 §5.1).
//!
//! Each participant of a community voice channel draws its own random
//! media secret and sends it to every other participant over a pairwise
//! channel; nobody else encrypts under it (RFC 9605 §4.4.1: "A given
//! base_key MUST NOT be used for encryption by multiple senders"). This is
//! how Signal group calls, MatrixRTC and Jitsi key media, and it is what
//! makes a split impossible: one writer per key.
//!
//! The pairwise channel is RFC 9180 HPKE in Auth mode between the two
//! members' community pseudonyms (the construction `mek::hpke_wrap_mek`
//! uses), under its own info string. The AAD binds the channel, the
//! recipient and the key index, so a sealed key cannot be replayed into
//! another channel, to another member, or as another index.

use ed25519_dalek::SigningKey;
use hkdf::Hkdf;
use rand::RngCore;
use rekindle_types::domains;
use rekindle_types::error::CryptoError;
use sha2::Sha512;
use zeroize::Zeroizing;

use crate::mek::{hpke_keys, HpkeAead, HpkeKdf, HpkeKem};

pub mod keyring;

/// A fresh random media secret.
#[must_use]
pub fn fresh() -> Zeroizing<[u8; 32]> {
    let mut secret = Zeroizing::new([0u8; 32]);
    rand::rngs::OsRng.fill_bytes(secret.as_mut());
    secret
}

/// A random starting key index for a channel session: 32 random bits, so
/// a rejoining participant's new indices never coincide with the ones a
/// receiver still holds from its previous session, and counting up from
/// it never wraps.
#[must_use]
pub fn random_session_index() -> u64 {
    u64::from(rand::rngs::OsRng.next_u32())
}

/// The AAD a sealed media key is bound to.
#[must_use]
pub fn seal_aad(channel_id: &str, recipient_hex: &str, key_index: u64) -> Vec<u8> {
    let mut aad = Vec::with_capacity(channel_id.len() + recipient_hex.len() + 10);
    aad.extend_from_slice(channel_id.as_bytes());
    aad.push(0);
    aad.extend_from_slice(recipient_hex.as_bytes());
    aad.push(0);
    aad.extend_from_slice(&key_index.to_be_bytes());
    aad
}

/// Seal our media `secret` to the member whose pseudonym public key is
/// `recipient_ed25519_public`, authenticated as `sender_signing_key`.
///
/// # Errors
/// A key that is not a valid Ed25519 point, or an HPKE failure.
pub fn seal(
    sender_signing_key: &SigningKey,
    recipient_ed25519_public: &[u8; 32],
    aad: &[u8],
    secret: &[u8; 32],
) -> Result<Vec<u8>, CryptoError> {
    use hpke::Serializable;
    let (our_sk, our_pk, their_pk) = hpke_keys(sender_signing_key, recipient_ed25519_public)?;
    let (enc, ciphertext) = hpke::single_shot_seal::<HpkeAead, HpkeKdf, HpkeKem>(
        &hpke::OpModeS::Auth((our_sk, our_pk)),
        &their_pk,
        domains::MEDIA_KEY_SEAL.as_bytes(),
        secret,
        aad,
    )
    .map_err(|e| CryptoError::Encryption(format!("HPKE seal: {e}")))?;
    let mut out = enc.to_bytes().to_vec();
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Open a media secret sealed to us by the member whose pseudonym public
/// key is `sender_ed25519_public`.
///
/// # Errors
/// Not sealed by that sender to us under this AAD, or malformed.
pub fn open(
    recipient_signing_key: &SigningKey,
    sender_ed25519_public: &[u8; 32],
    aad: &[u8],
    sealed: &[u8],
) -> Result<Zeroizing<[u8; 32]>, CryptoError> {
    use hpke::Deserializable;
    // 32 enc + 32 secret + 16 tag
    if sealed.len() != 80 {
        return Err(CryptoError::Decryption(
            "sealed media key is not 80 bytes".into(),
        ));
    }
    let (our_sk, _our_pk, their_pk) = hpke_keys(recipient_signing_key, sender_ed25519_public)?;
    let enc = <HpkeKem as hpke::Kem>::EncappedKey::from_bytes(&sealed[..32])
        .map_err(|e| CryptoError::Decryption(format!("HPKE encapped key: {e}")))?;
    let plaintext = Zeroizing::new(
        hpke::single_shot_open::<HpkeAead, HpkeKdf, HpkeKem>(
            &hpke::OpModeR::Auth(their_pk),
            &our_sk,
            &enc,
            domains::MEDIA_KEY_SEAL.as_bytes(),
            &sealed[32..],
            aad,
        )
        .map_err(|e| CryptoError::Decryption(format!("HPKE open: {e}")))?,
    );
    let mut secret = Zeroizing::new([0u8; 32]);
    if plaintext.len() != 32 {
        return Err(CryptoError::Decryption("media key is not 32 bytes".into()));
    }
    secret.copy_from_slice(&plaintext);
    Ok(secret)
}

/// A video sender's frame key: `HKDF-SHA512(salt = "", ikm = secret,
/// info = VIDEO_SENDER_KEY ‖ sender_key)`. Video seals whole frames with
/// its own AEAD, so it never uses the media secret itself, which SFrame
/// keys voice from.
#[must_use]
pub fn video_frame_key(secret: &[u8; 32], sender_key: &[u8]) -> Zeroizing<[u8; 32]> {
    let hk = Hkdf::<Sha512>::new(Some(&[]), secret);
    let mut info = Vec::with_capacity(domains::VIDEO_SENDER_KEY.len() + sender_key.len());
    info.extend_from_slice(domains::VIDEO_SENDER_KEY.as_bytes());
    info.extend_from_slice(sender_key);
    let mut out = Zeroizing::new([0u8; 32]);
    let _ = hk.expand(&info, out.as_mut());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(b: u8) -> SigningKey {
        SigningKey::from_bytes(&[b; 32])
    }

    #[test]
    fn a_sealed_key_opens_only_for_its_recipient_from_its_sender() {
        let (alice, bob, carol) = (key(1), key(2), key(3));
        let secret = fresh();
        let bob_hex = hex::encode(bob.verifying_key().to_bytes());
        let aad = seal_aad("ch", &bob_hex, 7);
        let sealed = seal(&alice, &bob.verifying_key().to_bytes(), &aad, &secret).unwrap();

        let opened = open(&bob, &alice.verifying_key().to_bytes(), &aad, &sealed).unwrap();
        assert_eq!(*opened, *secret);

        // Another recipient, another claimed sender, another binding: no.
        assert!(open(&carol, &alice.verifying_key().to_bytes(), &aad, &sealed).is_err());
        assert!(open(&bob, &carol.verifying_key().to_bytes(), &aad, &sealed).is_err());
        assert!(open(
            &bob,
            &alice.verifying_key().to_bytes(),
            &seal_aad("ch", &bob_hex, 8),
            &sealed
        )
        .is_err());
        assert!(open(
            &bob,
            &alice.verifying_key().to_bytes(),
            &seal_aad("other", &bob_hex, 7),
            &sealed
        )
        .is_err());
    }

    #[test]
    fn video_frame_keys_differ_per_sender_and_from_the_secret() {
        let secret = fresh();
        let a = video_frame_key(&secret, b"alice");
        let b = video_frame_key(&secret, b"bob");
        assert_ne!(*a, *b);
        assert_ne!(*a, *secret);
    }
}
