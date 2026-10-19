//! Fixtures for the ratchet and prekey tests: a seeded generator and a
//! party with its own database.

use std::sync::Arc;

use parking_lot::Mutex;
use rand::rngs::StdRng;
use rand::{RngCore, SeedableRng};
use rekindle_crypto::Identity;
use rekindle_utils::random::RandomSource;

use crate::ratchet::{LocalParty, PeerAddress};
use crate::Db;

/// A seeded generator, so a seed fixes every key a step draws.
pub struct Seeded(Mutex<StdRng>);

impl Seeded {
    pub fn new(seed: u64) -> Self {
        Self(Mutex::new(StdRng::seed_from_u64(seed)))
    }
}

impl rand::CryptoRng for Seeded {}

impl RandomSource for Seeded {
    fn fill(&self, dest: &mut [u8]) {
        self.0.lock().fill_bytes(dest);
    }
}

/// One side of a conversation: its database and its local party.
pub struct Party {
    pub db: Db,
    pub local: LocalParty,
}

impl Party {
    /// A fresh identity with an in-memory database holding its row.
    pub fn new(seed: u64) -> Self {
        let identity = Arc::new(Identity::generate());
        let conn = crate::repo::fixture::identity_conn(&identity.public_key_bytes());
        Self {
            db: Db::from(rekindle_asql::Connection::from(conn)),
            local: LocalParty {
                identity,
                rng: Arc::new(Seeded::new(seed)),
                now_ms: 1_000,
            },
        }
    }

    /// The address others reach this party at.
    pub fn address(&self) -> PeerAddress {
        PeerAddress::sole_device(self.local.owner())
    }

    /// The number of rows in `table` for this party.
    pub async fn count(&self, table: &'static str) -> i64 {
        self.db
            .call(move |c| c.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0)))
            .await
            .unwrap()
    }
}
