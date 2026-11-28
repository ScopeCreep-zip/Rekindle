//! A test community member's keys, shared by the governance tests: its
//! pseudonym and its member record owner `K_M`.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_secrets::member_record::dflt_opaque_record_key;
use rekindle_types::id::PseudonymKey;

use crate::device_signer::OpSigner;

/// A member's keys: its pseudonym, its record owner `K_M` and the op key
/// `d` of its device (ADR 0022 rule 1: `d` signs, the pseudonym authors).
pub(crate) struct Member {
    pub(crate) pseudonym: SigningKey,
    pub(crate) owner: SigningKey,
    pub(crate) device: SigningKey,
}

/// The op key of `p`'s device in these tests: its own key, never `p`
/// (ADR 0022 rule 1), fixed per pseudonym so a test's ops are stable.
pub(crate) fn device_of(p: &SigningKey) -> SigningKey {
    let mut h = blake3::Hasher::new();
    h.update(b"rekindle-test-device-op-key");
    h.update(p.as_bytes());
    SigningKey::from_bytes(h.finalize().as_bytes())
}

/// `p` authors, `device` signs.
pub(crate) fn signer<'a>(p: &SigningKey, device: &'a SigningKey) -> OpSigner<'a> {
    OpSigner {
        author: PseudonymKey(p.verifying_key().to_bytes()),
        device,
    }
}

impl Member {
    pub(crate) fn new(seed: u8) -> Self {
        let pseudonym = SigningKey::from_bytes(&[seed; 32]);
        Self {
            device: device_of(&pseudonym),
            pseudonym,
            owner: SigningKey::from_bytes(&[seed.wrapping_add(100); 32]),
        }
    }

    /// This member's op signer: its pseudonym authors, its device signs.
    pub(crate) fn signer(&self) -> OpSigner<'_> {
        signer(&self.pseudonym, &self.device)
    }

    pub(crate) fn author(&self) -> PseudonymKey {
        PseudonymKey(self.pseudonym.verifying_key().to_bytes())
    }

    pub(crate) fn owner_text(&self) -> String {
        rekindle_crypto::keypair_text::public_key_text(&self.owner.verifying_key().to_bytes())
    }

    /// The record key Veilid gives `K_M`'s DFLT(8) member record.
    pub(crate) fn record(&self) -> String {
        let opaque = dflt_opaque_record_key(
            &self.owner.verifying_key().to_bytes(),
            rekindle_types::dht_layout::member_record::SUBKEY_COUNT,
        );
        format!(
            "VLD0:{}:{}",
            URL_SAFE_NO_PAD.encode(opaque),
            URL_SAFE_NO_PAD.encode([5; 32])
        )
    }
}
