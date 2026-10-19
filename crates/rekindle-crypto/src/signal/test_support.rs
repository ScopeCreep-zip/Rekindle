//! Test fixtures for the pure session API: a seeded [`RandomSource`] and a
//! responder's prekey material with the bundles it publishes.

use parking_lot::Mutex;
use rand::rngs::StdRng;
use rand::{RngCore, SeedableRng};
use rekindle_secrets::pq_keys::MlKemSecret;
use rekindle_utils::random::RandomSource;
use x25519_dalek::{PublicKey as X25519Public, StaticSecret};

use crate::signal::pqxdh::agreement::{InitiatorAgreement, PqPrekeyId, ResponderKeys};
use crate::signal::pqxdh::verify::{pq_signing_payload, spk_signing_payload};
use crate::signal::prekeys::PreKeyBundle;
use crate::Identity;

/// A seeded generator, so a seed fixes every ratchet key a test mints.
pub struct TestRandom(Mutex<StdRng>);

impl TestRandom {
    pub fn new(seed: u64) -> Self {
        Self(Mutex::new(StdRng::seed_from_u64(seed)))
    }

    /// A generator seeded by a message's bytes: deterministic, and
    /// distinct for every message, so no two ratchet steps in a test draw
    /// the same key.
    pub fn for_message(bytes: &[u8]) -> Self {
        Self(Mutex::new(StdRng::from_seed(
            *blake3::hash(bytes).as_bytes(),
        )))
    }
}

impl rand::CryptoRng for TestRandom {}

impl RandomSource for TestRandom {
    fn fill(&self, dest: &mut [u8]) {
        self.0.lock().fill_bytes(dest);
    }
}

/// One handed-out set of one-time keys: a curve prekey and an ML-KEM key.
pub struct OneTime {
    pub opk_id: u32,
    pub opk: StaticSecret,
    pub pq_id: u32,
    pub pq: MlKemSecret,
}

/// A responder's identity and prekeys.
pub struct BobKeys {
    pub identity: Identity,
    pub spk: StaticSecret,
    pub pq_last_resort: MlKemSecret,
    pub one_time: Vec<OneTime>,
}

impl BobKeys {
    /// A responder with one set of one-time keys.
    pub fn new() -> Self {
        Self::with_one_time(1)
    }

    /// A responder with `count` sets of one-time keys, ids `100 + i` and
    /// `200 + i`.
    pub fn with_one_time(count: u32) -> Self {
        Self {
            identity: Identity::generate(),
            spk: StaticSecret::random_from_rng(rand::rngs::OsRng),
            pq_last_resort: MlKemSecret::generate().0,
            one_time: (0..count)
                .map(|i| OneTime {
                    opk_id: 100 + i,
                    opk: StaticSecret::random_from_rng(rand::rngs::OsRng),
                    pq_id: 200 + i,
                    pq: MlKemSecret::generate().0,
                })
                .collect(),
        }
    }

    /// The bundle with the first one-time set, or with none.
    pub fn bundle(&self, one_time: bool) -> PreKeyBundle {
        let mut bundle = self.long_lived_bundle();
        if one_time {
            self.add_one_time(&mut bundle, 0);
        }
        bundle
    }

    /// The bundle handed to one peer: one-time set `index`.
    pub fn handout(&self, index: usize) -> PreKeyBundle {
        let mut bundle = self.long_lived_bundle();
        self.add_one_time(&mut bundle, index);
        bundle
    }

    /// The secrets for an agreement's ids.
    pub fn responder_keys(&self, agreed: &InitiatorAgreement) -> ResponderKeys<'_> {
        self.keys_for(agreed.one_time_prekey_id, agreed.pq_prekey)
    }

    /// The secrets a PreKeyInit naming these ids needs.
    pub fn keys_for(&self, opk_id: Option<u32>, pq: PqPrekeyId) -> ResponderKeys<'_> {
        let pq_prekey = match pq {
            PqPrekeyId::OneTime(id) => {
                &self
                    .one_time
                    .iter()
                    .find(|set| set.pq_id == id)
                    .expect("a handed-out PQ id")
                    .pq
            }
            PqPrekeyId::LastResort(_) => &self.pq_last_resort,
        };
        let one_time_prekey = opk_id.map(|id| {
            &self
                .one_time
                .iter()
                .find(|set| set.opk_id == id)
                .expect("a handed-out one-time id")
                .opk
        });
        ResponderKeys {
            identity: &self.identity,
            signed_prekey: &self.spk,
            one_time_prekey,
            pq_prekey,
        }
    }

    fn long_lived_bundle(&self) -> PreKeyBundle {
        let spk = X25519Public::from(&self.spk);
        let lr = self.pq_last_resort.public();
        PreKeyBundle {
            identity_key: self.identity.public_key_bytes().to_vec(),
            signed_prekey: spk.as_bytes().to_vec(),
            signed_prekey_signature: self.sign(&spk_signing_payload(spk.as_bytes())),
            one_time_prekey: None,
            one_time_prekey_id: None,
            registration_id: 1,
            pqpk_lr: lr.as_bytes().to_vec(),
            pqpk_lr_signature: self.sign(&pq_signing_payload(b"LR", lr.as_bytes())),
            pqpk_ot: None,
            pqpk_ot_signature: None,
            pqpk_ot_id: None,
        }
    }

    fn add_one_time(&self, bundle: &mut PreKeyBundle, index: usize) {
        let set = &self.one_time[index];
        let pq = set.pq.public();
        bundle.one_time_prekey = Some(X25519Public::from(&set.opk).as_bytes().to_vec());
        bundle.one_time_prekey_id = Some(set.opk_id);
        bundle.pqpk_ot_signature = Some(self.sign(&pq_signing_payload(b"OT", pq.as_bytes())));
        bundle.pqpk_ot = Some(pq.as_bytes().to_vec());
        bundle.pqpk_ot_id = Some(set.pq_id);
    }

    fn sign(&self, payload: &[u8]) -> Vec<u8> {
        self.identity.sign(payload).to_bytes().to_vec()
    }
}
