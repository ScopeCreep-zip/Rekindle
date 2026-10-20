//! Pre-session frames (plan E2.3): a 1:1 kind sent before, or outside, a
//! ratchet session is HPKE Base-sealed to the recipient's identity X25519
//! and signed by the sender's Ed25519 identity over
//! `DM_PRESESSION_SIG ‖ recipient ‖ typeId ‖ enc ‖ ct`.
//!
//! Base mode authenticates no sender (RFC 9180 §9.8), and Veilid names
//! none, so the signature is RFC 9180 §9.1.1's own mitigation ("produce a
//! digital signature over (enc, ct) tuples using a sender's private key").
//! Binding the recipient keeps a signed frame from being re-addressed to
//! someone else; the HPKE AAD binds the sender and the typeId, so the
//! ciphertext opens only under the frame it was sealed into. Auth mode is
//! not used: it authenticates only an X25519 key and is open to
//! key-compromise impersonation (§9.1.1).
//!
//! The receiver checks the signature ([`verify`]) before anything is
//! decrypted ([`open`]), so a frame forged in another sender's name costs
//! one signature check and charges nothing to that sender.

use rekindle_types::domains;
use zeroize::Zeroizing;

use crate::{CryptoError, Identity};

/// A sealed frame's carrier fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sealed {
    /// The HPKE encapsulated key.
    pub enc: [u8; 32],
    /// The HPKE ciphertext.
    pub ciphertext: Vec<u8>,
    /// The sender's identity signature over [`signing_bytes`].
    pub signature: [u8; 64],
}

/// Why a sealed frame was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// The claimed sender is not an Ed25519 key.
    NotAKey,
    /// The signature does not verify under the sender for us.
    BadSignature,
    /// The ciphertext does not open to us under this kind.
    DoesNotOpen,
}

impl Refused {
    /// The refusal as a log reason.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotAKey => "sender is not a key",
            Self::BadSignature => "bad signature",
            Self::DoesNotOpen => "does not open",
        }
    }
}

/// What a sealed frame's sender signs.
#[must_use]
pub fn signing_bytes(
    recipient: &[u8; 32],
    type_id: u16,
    enc: &[u8; 32],
    ciphertext: &[u8],
) -> Vec<u8> {
    let label = domains::DM_PRESESSION_SIG.as_bytes();
    let mut bytes = Vec::with_capacity(label.len() + 66 + ciphertext.len());
    bytes.extend_from_slice(label);
    bytes.extend_from_slice(recipient);
    bytes.extend_from_slice(&type_id.to_be_bytes());
    bytes.extend_from_slice(enc);
    bytes.extend_from_slice(ciphertext);
    bytes
}

/// The HPKE AAD of a sealed frame: its sender and typeId.
#[must_use]
pub fn aad(sender: &[u8; 32], type_id: u16) -> [u8; 34] {
    let mut aad = [0u8; 34];
    aad[..32].copy_from_slice(sender);
    aad[32..].copy_from_slice(&type_id.to_be_bytes());
    aad
}

/// Seal `plaintext` from `identity` to `recipient` (an Ed25519 identity)
/// as a `type_id` frame under the HPKE `info` label of its kind, and sign
/// it.
///
/// # Errors
/// `recipient` is not a curve point, or HPKE failed.
pub fn seal(
    identity: &Identity,
    recipient: &[u8; 32],
    type_id: u16,
    info: &str,
    plaintext: &[u8],
) -> Result<Sealed, CryptoError> {
    let recipient_x25519 = Identity::peer_ed25519_to_x25519(recipient)?;
    let sealed = rekindle_secrets::hpke::seal_base(
        recipient_x25519.as_bytes(),
        info,
        &aad(&identity.public_key_bytes(), type_id),
        plaintext,
    )?;
    let signature = identity
        .sign(&signing_bytes(
            recipient,
            type_id,
            &sealed.enc,
            &sealed.ciphertext,
        ))
        .to_bytes();
    Ok(Sealed {
        enc: sealed.enc,
        ciphertext: sealed.ciphertext,
        signature,
    })
}

/// Whether `sealed`, claimed by `sender` as a `type_id` frame, is signed by
/// `sender` for `recipient` (our identity).
///
/// # Errors
/// The sender is not a key, or the signature does not verify.
pub fn verify(
    recipient: &[u8; 32],
    sender: &[u8; 32],
    type_id: u16,
    sealed: &Sealed,
) -> Result<(), Refused> {
    let key = ed25519_dalek::VerifyingKey::from_bytes(sender).map_err(|_| Refused::NotAKey)?;
    let signing = signing_bytes(recipient, type_id, &sealed.enc, &sealed.ciphertext);
    Identity::verify(
        &key,
        &signing,
        &ed25519_dalek::Signature::from_bytes(&sealed.signature),
    )
    .map_err(|_| Refused::BadSignature)
}

/// Open `sealed` from `sender` to `identity` as a `type_id` frame under
/// `info`. Call only after [`verify`].
///
/// # Errors
/// The ciphertext does not open.
pub fn open(
    identity: &Identity,
    sender: &[u8; 32],
    type_id: u16,
    info: &str,
    sealed: &Sealed,
) -> Result<Zeroizing<Vec<u8>>, Refused> {
    let secret = Zeroizing::new(identity.to_x25519_secret().to_bytes());
    rekindle_secrets::hpke::open_base(
        &secret,
        info,
        &aad(sender, type_id),
        &sealed.enc,
        &sealed.ciphertext,
    )
    .map_err(|_| Refused::DoesNotOpen)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KIND: u16 = 0x0102;
    const INFO: &str = domains::DM_FRIEND_REJECT_HPKE;

    #[test]
    fn a_sealed_frame_verifies_and_opens_only_for_its_recipient() {
        let (alice, bob, carol) = (
            Identity::generate(),
            Identity::generate(),
            Identity::generate(),
        );
        let (a, b) = (alice.public_key_bytes(), bob.public_key_bytes());
        let sealed = seal(&alice, &b, KIND, INFO, b"hello").unwrap();
        verify(&b, &a, KIND, &sealed).unwrap();
        assert_eq!(
            open(&bob, &a, KIND, INFO, &sealed).unwrap().as_slice(),
            b"hello"
        );
        // Re-addressed, relabelled, or claimed by another sender.
        let c = carol.public_key_bytes();
        assert_eq!(verify(&c, &a, KIND, &sealed), Err(Refused::BadSignature));
        assert_eq!(verify(&b, &a, 0x0009, &sealed), Err(Refused::BadSignature));
        assert_eq!(verify(&b, &c, KIND, &sealed), Err(Refused::BadSignature));
        assert_eq!(
            open(&carol, &a, KIND, INFO, &sealed),
            Err(Refused::DoesNotOpen)
        );
        assert_eq!(
            open(&bob, &c, KIND, INFO, &sealed),
            Err(Refused::DoesNotOpen)
        );
        assert_eq!(
            open(&bob, &a, KIND, domains::DM_FRIEND_REQUEST_HPKE, &sealed),
            Err(Refused::DoesNotOpen)
        );
    }

    #[test]
    fn a_tampered_ciphertext_fails_the_signature() {
        let (alice, bob) = (Identity::generate(), Identity::generate());
        let (a, b) = (alice.public_key_bytes(), bob.public_key_bytes());
        let mut sealed = seal(&alice, &b, KIND, INFO, b"hello").unwrap();
        sealed.ciphertext[0] ^= 1;
        assert_eq!(verify(&b, &a, KIND, &sealed), Err(Refused::BadSignature));
    }
}
