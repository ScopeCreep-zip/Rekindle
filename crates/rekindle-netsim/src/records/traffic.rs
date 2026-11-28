//! Other apps' DHT traffic (plan E3.2 S7 record warming): records a node
//! creates and writes, each admitted at the stores it lands on, which evict
//! their least recently used past `REMOTE_MAX_RECORDS` ([`super::stores`],
//! after 0.5.8 `record_index.rs:177-203`). Nothing here changes how a store
//! keeps or evicts a record; it only drives the creates a busy network makes.

use std::sync::Arc;

use rekindle_records::port::{DhtBackend, RecordSchema};

use super::dht::SimDht;
use super::stores::REMOTE_MAX_RECORDS;
use crate::{Netsim, SimRandom};

impl Netsim {
    /// Node `from` creates and writes fresh DFLT(1) records until no store
    /// still holds a record it last used before this call: a record nobody
    /// read, wrote or rehydrated meanwhile is evicted from every store, and
    /// one a node used meanwhile stays. The count made.
    ///
    /// # Panics
    /// A create or set failed, or the stores never turned over.
    pub async fn store_traffic(&self, from: usize) -> usize {
        let start = self.net.inner.lock().lru;
        let rng = Arc::new(SimRandom::new(self.seed ^ 0x7a1f_f1c0, from));
        let dht = SimDht::new(from, Arc::clone(&self.net), rng);
        let cap = self.net.inner.lock().nodes.len() * super::REPLICAS * REMOTE_MAX_RECORDS;
        let mut made = 0;
        while self.held_unused_since(start) {
            assert!(made < cap, "the stores did not turn over in {cap} records");
            let created = dht.create(&RecordSchema::Dflt { owner_subkeys: 1 }, None);
            let key = created.await.expect("traffic create").key;
            dht.set(&key, 0, vec![0], None).await.expect("traffic set");
            made += 1;
        }
        made
    }

    /// Whether a store holds a value of a record it last used at or before
    /// LRU stamp `start`.
    fn held_unused_since(&self, start: u64) -> bool {
        self.net.inner.lock().records.values().any(|record| {
            (record.replicas.iter())
                .any(|r| r.used <= start && r.values.iter().any(Option::is_some))
        })
    }
}
