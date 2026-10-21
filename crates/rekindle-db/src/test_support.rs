//! Fixtures for the ratchet and prekey tests: a seeded generator, a party
//! with its own database, and a bundle assembled from a party's published
//! prekey record the way an initiator assembles it (plan E2.4).

use std::sync::Arc;

use parking_lot::Mutex;
use rand::rngs::StdRng;
use rand::{RngCore, SeedableRng};
use rekindle_codec::capnp_codec::identity::{decode_opk_slot, decode_prekey_head};
use rekindle_crypto::signal::prekey_record::mint_set;
use rekindle_crypto::signal::PreKeyBundle;
use rekindle_crypto::Identity;
use rekindle_utils::random::RandomSource;

use rusqlite::Connection;

use crate::prekeys;
use crate::ratchet::{LocalParty, PeerAddress};
use crate::Db;
use rekindle_crypto::signal::prekey_record::Publish;

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
        let conn = schema_conn(&crate::repo::identity_text(&identity.public_key_bytes()));
        Self {
            db: Db::from(rekindle_asql::Connection::from(conn)),
            local: LocalParty {
                identity,
                rng: Arc::new(Seeded::new(seed)),
                now_ms: 1_000,
            },
        }
    }

    /// A fresh identity whose database is the file `path`, opened the way
    /// a host opens it (`crate::open`: WAL and its settings).
    pub async fn on_disk(seed: u64, path: &std::path::Path) -> Self {
        let identity = Arc::new(Identity::generate());
        let db = crate::open(path, &[7; 32]).unwrap();
        let owner = crate::repo::identity_text(&identity.public_key_bytes());
        db.call(move |c| {
            crate::repo::identity::insert(c, &owner, "me", 0, "aaaaaaaaaaaaaaaaaaaaaaaaaa")
        })
        .await
        .unwrap();
        Self {
            db,
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

    /// This party's prekey record as published now, minting and storing
    /// a whole one first when it has none.
    pub async fn record(&self) -> Vec<Publish> {
        let owner = self.local.owner();
        let local = prekeys::local_prekeys(&self.db, &owner).await.unwrap();
        if local.head.is_none() {
            let set = mint_set(
                &self.local.identity,
                self.local.rng.as_ref(),
                self.local.now_ms,
            );
            prekeys::store_set(&self.db, &owner, set).await.unwrap();
        }
        prekeys::published_values(&self.db, &self.local)
            .await
            .unwrap()
    }

    /// A bundle for this party from its record now: the head and the slot
    /// at `subkey`, or the head alone.
    pub async fn bundle(&self, subkey: Option<u32>) -> PreKeyBundle {
        bundle_from(&self.local.owner(), &self.record().await, subkey)
    }

    /// One integer `sql` reads from this party's database.
    pub async fn scalar(&self, sql: &'static str) -> i64 {
        self.db
            .call(move |c| c.query_row(sql, [], |r| r.get(0)))
            .await
            .unwrap()
    }

    /// The number of rows in `table` for this party.
    pub async fn count(&self, table: &'static str) -> i64 {
        self.db
            .call(move |c| c.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0)))
            .await
            .unwrap()
    }
}

/// An in-memory database with the schema and an identity row for
/// `public_key`, which the per-owner tables reference.
///
/// # Panics
/// The schema does not apply (a broken `001_init.sql`).
#[must_use]
pub fn schema_conn(public_key: &str) -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
    conn.execute_batch(crate::SCHEMA).unwrap();
    conn.execute(
        "INSERT INTO identity (public_key, display_name, created_at, friend_request_token) VALUES (?1, 'me', 0, 'aaaaaaaaaaaaaaaaaaaaaaaaaa')",
        [public_key],
    )
    .unwrap();
    conn
}

/// The bundle an initiator assembles for `owner` from its published
/// `values`: the head and the slot at `subkey`, or the head alone.
///
/// # Panics
/// The values hold no head, no slot at `subkey`, or do not verify.
#[must_use]
pub fn bundle_from(owner: &[u8; 32], values: &[Publish], subkey: Option<u32>) -> PreKeyBundle {
    let value = |at: u32| {
        &values
            .iter()
            .find(|p| p.subkey == at)
            .unwrap_or_else(|| panic!("no value at subkey {at}"))
            .value
    };
    let head = decode_prekey_head(value(rekindle_types::dht_layout::prekeys::HEAD)).unwrap();
    let slot = subkey.map(|at| decode_opk_slot(value(at)).unwrap());
    // Read at the head's own time: the age bound always holds.
    PreKeyBundle::from_record(owner, &head, slot.as_ref(), head.issued_at).unwrap()
}
