//! A test community member's keys, shared by the governance tests: its
//! pseudonym and its member record owner `K_M`.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_secrets::member_record::dflt_opaque_record_key;
use rekindle_types::id::PseudonymKey;

/// A member's keys: its pseudonym and its record owner `K_M`.
pub(crate) struct Member {
    pub(crate) pseudonym: SigningKey,
    pub(crate) owner: SigningKey,
}

impl Member {
    pub(crate) fn new(seed: u8) -> Self {
        Self {
            pseudonym: SigningKey::from_bytes(&[seed; 32]),
            owner: SigningKey::from_bytes(&[seed.wrapping_add(100); 32]),
        }
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
