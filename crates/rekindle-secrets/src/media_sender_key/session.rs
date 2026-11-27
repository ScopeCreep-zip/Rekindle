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

use crate::hpke;

/// The media secret a sealed key carries.
const SECRET_LEN: usize = 32;
/// 32 enc + 32 secret + 16 tag.
const SEALED_LEN: usize = hpke::ENC_LEN + SECRET_LEN + hpke::TAG_LEN;

/// Our X25519 key for one channel session. The secret is erased on drop.
pub struct SessionKey {
    secret: Zeroizing<[u8; 32]>,
    public: [u8; 32],
}

impl SessionKey {
    /// A fresh session key.
    #[must_use]
    pub fn fresh() -> Self {
        let mut secret = Zeroizing::new([0u8; 32]);
        rand::rngs::OsRng.fill_bytes(secret.as_mut());
        let public = hpke::public_key(&secret);
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
        if sealed.len() != SEALED_LEN {
            return Err(CryptoError::Decryption(
                "sealed media key is not 80 bytes".into(),
            ));
        }
        let (enc, ciphertext) = sealed.split_at(hpke::ENC_LEN);
        let plaintext =
            hpke::open_base(&self.secret, domains::MEDIA_KEY_SEAL, aad, enc, ciphertext)?;
        if plaintext.len() != SECRET_LEN {
            return Err(CryptoError::Decryption("media key is not 32 bytes".into()));
        }
        let mut secret = Zeroizing::new([0u8; SECRET_LEN]);
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
    let recipient: [u8; 32] = recipient_session
        .try_into()
        .map_err(|_| CryptoError::InvalidKey("recipient session key is not 32 bytes".into()))?;
    let sealed = hpke::seal_base(&recipient, domains::MEDIA_KEY_SEAL, aad, secret)?;
    let mut out = Vec::with_capacity(SEALED_LEN);
    out.extend_from_slice(&sealed.enc);
    out.extend_from_slice(&sealed.ciphertext);
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
#[path = "session_tests.rs"]
mod tests;
