//! Per-session keys that call-media keys are sealed to (plan C7.29 G2).
//!
//! HPKE to a long-term key has no forward secrecy against that key's
//! compromise (RFC 9180 §9.1): a recorded `VoiceMediaKey` sealed to a
//! community pseudonym opens for anyone who later learns the pseudonym's
//! secret. So each participant draws an X25519 key per channel session,
//! announces its public half in the join handshake, signed by its
//! pseudonym, and erases the secret when the session ends
//! (`keyring::ChannelSenderKeyStore::end`). Keys sealed to it then stay
//! closed once the session is over, as DAVE's single-use key packages and
//! Jitsi's per-conference Olm sessions do.
//!
//! A sealed key is HPKE Base mode to the recipient's session key, plus the
//! sender's pseudonym signature over channel, recipient, index and the
//! sealed bytes ([`sealed_key_message`]): Base mode does not authenticate
//! the sender, the signature does.

use ed25519_dalek::{Signature, VerifyingKey};
use rand::RngCore;
use rekindle_types::domains;
use rekindle_types::error::CryptoError;
use zeroize::Zeroizing;

use crate::mek::{HpkeAead, HpkeKdf, HpkeKem};

/// 32 enc + 32 secret + 16 tag.
const SEALED_LEN: usize = 80;

/// Our X25519 key for one channel session. The secret is erased on drop.
pub struct SessionKey {
    secret: Zeroizing<[u8; 32]>,
    public: [u8; 32],
}

impl SessionKey {
    /// A fresh session key.
    #[must_use]
    pub fn fresh() -> Self {
        use hpke::{Deserializable, Serializable};
        let mut secret = Zeroizing::new([0u8; 32]);
        rand::rngs::OsRng.fill_bytes(secret.as_mut());
        let sk = <HpkeKem as hpke::Kem>::PrivateKey::from_bytes(secret.as_ref())
            .expect("any 32 bytes are an X25519 private key");
        let mut public = [0u8; 32];
        public.copy_from_slice(&<HpkeKem as hpke::Kem>::sk_to_pk(&sk).to_bytes());
        Self { secret, public }
    }

    /// The public half, announced in the join handshake.
    #[must_use]
    pub fn public(&self) -> [u8; 32] {
        self.public
    }

    /// Open a media secret sealed to this key under `aad`.
    ///
    /// # Errors
    /// Not sealed to this key under this AAD, or malformed.
    pub fn open(&self, aad: &[u8], sealed: &[u8]) -> Result<Zeroizing<[u8; 32]>, CryptoError> {
        use hpke::Deserializable;
        if sealed.len() != SEALED_LEN {
            return Err(CryptoError::Decryption(
                "sealed media key is not 80 bytes".into(),
            ));
        }
        let sk = <HpkeKem as hpke::Kem>::PrivateKey::from_bytes(self.secret.as_ref())
            .map_err(|e| CryptoError::InvalidKey(format!("session key: {e}")))?;
        let enc = <HpkeKem as hpke::Kem>::EncappedKey::from_bytes(&sealed[..32])
            .map_err(|e| CryptoError::Decryption(format!("HPKE encapped key: {e}")))?;
        let plaintext = Zeroizing::new(
            hpke::single_shot_open::<HpkeAead, HpkeKdf, HpkeKem>(
                &hpke::OpModeR::Base,
                &sk,
                &enc,
                domains::MEDIA_KEY_SEAL.as_bytes(),
                &sealed[32..],
                aad,
            )
            .map_err(|e| CryptoError::Decryption(format!("HPKE open: {e}")))?,
        );
        if plaintext.len() != 32 {
            return Err(CryptoError::Decryption("media key is not 32 bytes".into()));
        }
        let mut secret = Zeroizing::new([0u8; 32]);
        secret.copy_from_slice(&plaintext);
        Ok(secret)
    }
}

/// Seal our media `secret` to a participant's session key under `aad`.
///
/// # Errors
/// A session key that is not 32 bytes, or an HPKE failure.
pub fn seal(
    recipient_session: &[u8],
    aad: &[u8],
    secret: &[u8; 32],
) -> Result<Vec<u8>, CryptoError> {
    use hpke::{Deserializable, Serializable};
    let pk = <HpkeKem as hpke::Kem>::PublicKey::from_bytes(recipient_session)
        .map_err(|e| CryptoError::InvalidKey(format!("recipient session key: {e}")))?;
    let (enc, ciphertext) = hpke::single_shot_seal::<HpkeAead, HpkeKdf, HpkeKem>(
        &hpke::OpModeS::Base,
        &pk,
        domains::MEDIA_KEY_SEAL.as_bytes(),
        secret,
        aad,
    )
    .map_err(|e| CryptoError::Encryption(format!("HPKE seal: {e}")))?;
    let mut out = enc.to_bytes().to_vec();
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

fn message(domain: &str, parts: &[&[u8]]) -> Vec<u8> {
    let mut out = domain.as_bytes().to_vec();
    for part in parts {
        out.push(0);
        out.extend_from_slice(part);
    }
    out
}

/// What a participant's pseudonym signs to announce its session key.
#[must_use]
pub fn session_key_message(community_id: &str, channel_id: &str, session_key: &[u8]) -> Vec<u8> {
    message(
        domains::MEDIA_SESSION_KEY,
        &[community_id.as_bytes(), channel_id.as_bytes(), session_key],
    )
}

/// What a sender's pseudonym signs over a sealed media key.
#[must_use]
pub fn sealed_key_message(
    channel_id: &str,
    recipient_hex: &str,
    key_index: u64,
    sealed: &[u8],
) -> Vec<u8> {
    message(
        domains::MEDIA_KEY_SIG,
        &[
            channel_id.as_bytes(),
            recipient_hex.as_bytes(),
            &key_index.to_be_bytes(),
            sealed,
        ],
    )
}

/// Whether `signature` is the pseudonym `signer_hex`'s over `message`.
#[must_use]
pub fn verify_pseudonym(signer_hex: &str, message: &[u8], signature: &[u8]) -> bool {
    let Some(key) = hex::decode(signer_hex)
        .ok()
        .and_then(|b| <[u8; 32]>::try_from(b).ok())
        .and_then(|b| VerifyingKey::from_bytes(&b).ok())
    else {
        return false;
    };
    let Ok(signature) = Signature::from_slice(signature) else {
        return false;
    };
    key.verify_strict(message, &signature).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    #[test]
    fn a_sealed_key_opens_only_with_its_session_key_and_binding() {
        let (bob, carol) = (SessionKey::fresh(), SessionKey::fresh());
        let secret = super::super::fresh();
        let sealed = seal(&bob.public(), b"aad", &secret).unwrap();
        assert_eq!(*bob.open(b"aad", &sealed).unwrap(), *secret);
        assert!(carol.open(b"aad", &sealed).is_err());
        assert!(bob.open(b"other", &sealed).is_err());
    }

    /// Once the session key is gone nothing recorded opens: no long-term
    /// secret reaches a sealed key (RFC 9180 §9.1).
    #[test]
    fn a_recorded_key_does_not_open_after_the_session_ends() {
        let bob = SessionKey::fresh();
        let sealed = seal(&bob.public(), b"aad", &super::super::fresh()).unwrap();
        drop(bob);
        // A new session key, or any other key, is not the erased one.
        assert!(SessionKey::fresh().open(b"aad", &sealed).is_err());
    }

    #[test]
    fn signatures_bind_signer_and_message() {
        let alice = SigningKey::from_bytes(&[1; 32]);
        let alice_hex = hex::encode(alice.verifying_key().to_bytes());
        let msg = sealed_key_message("ch", "bob", 7, b"sealed");
        let sig = alice.sign(&msg).to_bytes();
        assert!(verify_pseudonym(&alice_hex, &msg, &sig));
        assert!(!verify_pseudonym(
            &alice_hex,
            &sealed_key_message("ch", "bob", 8, b"sealed"),
            &sig
        ));
        assert!(!verify_pseudonym(&"00".repeat(32), &msg, &sig));
        assert!(!verify_pseudonym(&alice_hex, &msg, b"short"));
    }
}
