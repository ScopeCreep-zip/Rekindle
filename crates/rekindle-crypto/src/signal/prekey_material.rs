//! Prekey material (plan E2.3, E2.4): minting the keys
//! `rekindle_db::prekeys` keeps in `signal_prekeys`, and turning the
//! stored secrets a PreKeyInit names back into [`ResponderKeys`]. The
//! database holds bytes; every key operation stays in this crate. The
//! record values these keys are published in are signed in
//! `super::prekey_record`.

use rekindle_secrets::pq_keys::MlKemSecret;
use rekindle_utils::random::RandomSource;
use x25519_dalek::{PublicKey as X25519Public, StaticSecret};
use zeroize::Zeroizing;

use crate::bytes::to_32;
use crate::error::CryptoError;
use crate::signal::pqxdh::agreement::ResponderKeys;
use crate::Identity;

/// A freshly minted prekey: its secret, as stored, and its public half.
pub struct MintedPrekey {
    pub secret: Zeroizing<Vec<u8>>,
    pub public: Vec<u8>,
}

/// An X25519 prekey (signed or one-time) drawn from `rng`.
#[must_use]
pub fn mint_curve(rng: &dyn RandomSource) -> MintedPrekey {
    let mut bytes = Zeroizing::new([0u8; 32]);
    rng.fill(&mut *bytes);
    let secret = StaticSecret::from(*bytes);
    MintedPrekey {
        public: X25519Public::from(&secret).as_bytes().to_vec(),
        secret: Zeroizing::new(secret.to_bytes().to_vec()),
    }
}

/// An ML-KEM-768 prekey (last-resort or one-time), its 64-byte key
/// generation seed drawn from `rng` (plan E2.4: a seeded simulation mints
/// the same record).
#[must_use]
pub fn mint_pq(rng: &dyn RandomSource) -> MintedPrekey {
    let mut seed = Zeroizing::new([0u8; rekindle_secrets::pq_keys::ML_KEM_KEYGEN_SEED_BYTES]);
    rng.fill(&mut *seed);
    let (secret, public) = MlKemSecret::from_seed(&seed);
    MintedPrekey {
        secret: Zeroizing::new(secret.as_secret_bytes().to_vec()),
        public: public.as_bytes().to_vec(),
    }
}

/// The stored secrets one PreKeyInit names, parsed.
pub struct ResponderSecrets {
    signed: StaticSecret,
    one_time: Option<StaticSecret>,
    pq: MlKemSecret,
}

impl ResponderSecrets {
    /// Parse the stored secrets: the signed prekey, the one-time curve
    /// prekey when the init names one, and the ML-KEM key it names.
    ///
    /// # Errors
    /// A secret has the wrong length.
    pub fn from_bytes(
        signed_prekey: &[u8],
        one_time_prekey: Option<&[u8]>,
        pq_prekey: &[u8],
    ) -> Result<Self, CryptoError> {
        let curve = |bytes: &[u8], what| to_32(bytes, what).map(StaticSecret::from);
        Ok(Self {
            signed: curve(signed_prekey, "signed prekey")?,
            one_time: one_time_prekey
                .map(|bytes| curve(bytes, "one-time prekey"))
                .transpose()?,
            pq: MlKemSecret::from_secret_bytes(pq_prekey)
                .ok_or_else(|| CryptoError::invalid_key("ML-KEM prekey wrong length".into()))?,
        })
    }

    /// The keys [`crate::signal::SessionRecord::insert_from_init`] takes.
    #[must_use]
    pub fn keys<'a>(&'a self, identity: &'a Identity) -> ResponderKeys<'a> {
        ResponderKeys {
            identity,
            signed_prekey: &self.signed,
            one_time_prekey: self.one_time.as_ref(),
            pq_prekey: &self.pq,
        }
    }
}

#[cfg(test)]
#[path = "prekey_material_tests.rs"]
mod tests;
