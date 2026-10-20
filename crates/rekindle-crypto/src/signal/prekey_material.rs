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
mod tests {
    use super::*;
    use crate::signal::prekey_record::{mint_head, mint_slot};
    use crate::signal::test_support::TestRandom;
    use crate::signal::{PreKeyBundle, SessionRecord};

    /// A record minted here runs a session whose responder keys come
    /// back from the stored bytes.
    #[test]
    fn a_minted_record_runs_a_session_from_stored_secrets() {
        let bob = Identity::generate();
        let alice = Identity::generate();
        let rng = TestRandom::new(1);
        let head = mint_head(&bob, &rng, 5, 6, 1_000);
        let slot = mint_slot(&bob, &rng, 1, 7, 9);
        let bundle =
            PreKeyBundle::from_record(&bob.public_key_bytes(), &head.record, Some(&slot.record))
                .unwrap();
        let (_, init) = SessionRecord::new(bob.public_key_bytes())
            .initiate(&alice, &bundle, b"hi", b"", &rng)
            .unwrap();
        assert_eq!(init.one_time_prekey_id, Some(7));
        assert_eq!(init.signed_prekey_id, 5);

        let secrets = ResponderSecrets::from_bytes(
            &head.spk.secret,
            Some(&slot.opk.secret),
            &slot.pq_opk.secret,
        )
        .unwrap();
        let (_, first) = SessionRecord::new(alice.public_key_bytes())
            .insert_from_init(&init, &secrets.keys(&bob), b"", &rng)
            .unwrap();
        assert_eq!(first, b"hi");
    }

    #[test]
    fn short_secrets_are_refused() {
        assert!(
            ResponderSecrets::from_bytes(&[1; 31], None, &mint_pq(&TestRandom::new(3)).secret)
                .is_err()
        );
        assert!(ResponderSecrets::from_bytes(&[1; 32], None, &[0; 10]).is_err());
    }
}
