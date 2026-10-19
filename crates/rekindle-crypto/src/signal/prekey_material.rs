//! Prekey material for the stored prekeys (plan E2.3): minting the keys
//! `rekindle_db::prekeys` keeps in `signal_prekeys`, signing the bundle
//! built from them, and turning the stored secrets a PreKeyInit names back
//! into [`ResponderKeys`]. The database holds bytes; every key operation
//! stays in this crate.
//!
//! The bundle is the one `session::bundles` built (PQXDH §3.2–§3.3): the
//! signed prekey and the PQ last-resort key, each signed by the identity,
//! plus, in a bundle handed to one peer, a one-time curve prekey and a
//! signed one-time ML-KEM key.

use rekindle_secrets::pq_keys::MlKemSecret;
use rekindle_utils::random::RandomSource;
use x25519_dalek::{PublicKey as X25519Public, StaticSecret};
use zeroize::Zeroizing;

use crate::bytes::to_32;
use crate::error::CryptoError;
use crate::signal::pqxdh::agreement::ResponderKeys;
use crate::signal::pqxdh::verify::{pq_signing_payload, spk_signing_payload};
use crate::signal::prekeys::PreKeyBundle;
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

/// An ML-KEM-768 prekey (last-resort or one-time). Its randomness stays
/// `OsRng` inside `rekindle-secrets` (plan E2.2, C9.2's scope).
#[must_use]
pub fn mint_pq() -> MintedPrekey {
    let (secret, public) = MlKemSecret::generate();
    MintedPrekey {
        secret: Zeroizing::new(secret.as_secret_bytes().to_vec()),
        public: public.as_bytes().to_vec(),
    }
}

/// The one-time keys of a bundle handed to one peer.
pub struct OneTimePublics<'a> {
    pub opk_id: u32,
    pub opk: &'a [u8],
    pub pq_id: u32,
    pub pq: &'a [u8],
}

/// The bundle `identity` publishes over these public keys, each signed
/// under its domain tag.
#[must_use]
pub fn signed_bundle(
    identity: &Identity,
    spk: &[u8],
    pq_last_resort: &[u8],
    one_time: Option<&OneTimePublics<'_>>,
) -> PreKeyBundle {
    let public = identity.public_key_bytes();
    let sign = |payload: &[u8]| identity.sign(payload).to_bytes().to_vec();
    PreKeyBundle {
        identity_key: public.to_vec(),
        signed_prekey: spk.to_vec(),
        signed_prekey_signature: sign(&spk_signing_payload(spk)),
        one_time_prekey: one_time.map(|o| o.opk.to_vec()),
        one_time_prekey_id: one_time.map(|o| o.opk_id),
        registration_id: registration_id(&public),
        pqpk_lr: pq_last_resort.to_vec(),
        pqpk_lr_signature: sign(&pq_signing_payload(b"LR", pq_last_resort)),
        pqpk_ot: one_time.map(|o| o.pq.to_vec()),
        pqpk_ot_signature: one_time.map(|o| sign(&pq_signing_payload(b"OT", o.pq))),
        pqpk_ot_id: one_time.map(|o| o.pq_id),
    }
}

/// The bundle's registration id, derived from the identity so it is
/// stable: the first four key bytes, little-endian (the hosts' derivation
/// since D1). No PQXDH step reads it; E2.4 drops it from the bundle.
#[must_use]
pub fn registration_id(identity_public: &[u8; 32]) -> u32 {
    u32::from_le_bytes([
        identity_public[0],
        identity_public[1],
        identity_public[2],
        identity_public[3],
    ])
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
    use crate::signal::test_support::TestRandom;
    use crate::signal::SessionRecord;

    /// A bundle signed here runs a session whose responder keys come back
    /// from the stored bytes.
    #[test]
    fn a_signed_bundle_runs_a_session_from_stored_secrets() {
        let bob = Identity::generate();
        let alice = Identity::generate();
        let rng = TestRandom::new(1);
        let (spk, lr, opk, pq) = (mint_curve(&rng), mint_pq(), mint_curve(&rng), mint_pq());
        let one_time = OneTimePublics {
            opk_id: 7,
            opk: &opk.public,
            pq_id: 9,
            pq: &pq.public,
        };
        let bundle = signed_bundle(&bob, &spk.public, &lr.public, Some(&one_time));
        assert_eq!(
            bundle.registration_id,
            registration_id(&bob.public_key_bytes())
        );
        let (_, init) = SessionRecord::new(bob.public_key_bytes())
            .initiate(&alice, &bundle, b"hi", b"", &rng)
            .unwrap();
        assert_eq!(init.one_time_prekey_id, Some(7));

        let secrets =
            ResponderSecrets::from_bytes(&spk.secret, Some(&opk.secret), &pq.secret).unwrap();
        let (_, first) = SessionRecord::new(alice.public_key_bytes())
            .insert_from_init(&init, &secrets.keys(&bob), b"", &rng)
            .unwrap();
        assert_eq!(first, b"hi");
    }

    #[test]
    fn a_long_lived_bundle_has_no_one_time_keys_and_short_secrets_are_refused() {
        let bob = Identity::generate();
        let rng = TestRandom::new(2);
        let bundle = signed_bundle(&bob, &mint_curve(&rng).public, &mint_pq().public, None);
        assert!(bundle.one_time_prekey.is_none() && bundle.pqpk_ot.is_none());
        assert!(ResponderSecrets::from_bytes(&[1; 31], None, &mint_pq().secret).is_err());
        assert!(ResponderSecrets::from_bytes(&[1; 32], None, &[0; 10]).is_err());
    }
}
