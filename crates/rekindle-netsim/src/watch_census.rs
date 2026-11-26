//! What a scenario reads of the simulated watches (plan E3.2 S7 Verify:
//! "N = 40 shows <= 4 watchers per `M` and zero `Rejected`").

use std::collections::{BTreeMap, BTreeSet};

use rekindle_records::port::RecordSchema;

use crate::Netsim;

impl Netsim {
    /// Per `DFLT(subkeys)` record, the nodes holding a watch of it on any
    /// replica, by record key.
    #[must_use]
    pub fn watchers_of_dflt(&self, subkeys: u16) -> BTreeMap<String, BTreeSet<usize>> {
        let inner = self.net.inner.lock();
        inner
            .records
            .iter()
            .filter(|(_, r)| matches!(r.schema, RecordSchema::Dflt { owner_subkeys } if owner_subkeys == subkeys))
            .map(|(key, r)| {
                let nodes = r
                    .replicas
                    .iter()
                    .flat_map(|x| x.watches.iter().map(|w| w.node))
                    .collect();
                (key.clone(), nodes)
            })
            .collect()
    }

    /// The records `node` holds a watch of on any replica, with their
    /// schemas, by record key.
    #[must_use]
    pub fn watched_by(&self, node: usize) -> BTreeMap<String, RecordSchema> {
        let inner = self.net.inner.lock();
        inner
            .records
            .iter()
            .filter(|(_, r)| {
                r.replicas
                    .iter()
                    .any(|x| x.watches.iter().any(|w| w.node == node))
            })
            .map(|(key, r)| (key.clone(), r.schema.clone()))
            .collect()
    }

    /// The subkeys `node` watches of the record `key`, over every replica
    /// (empty when it holds no watch of it).
    #[must_use]
    pub fn watched_subkeys(&self, node: usize, key: &str) -> BTreeSet<u32> {
        let inner = self.net.inner.lock();
        inner
            .records
            .get(key)
            .into_iter()
            .flat_map(|r| r.replicas.iter())
            .flat_map(|x| x.watches.iter().filter(|w| w.node == node))
            .flat_map(|w| w.subkeys.iter().copied())
            .collect()
    }

    /// Watches a replica refused at its watch limit, since the start.
    #[must_use]
    pub fn watch_refusals(&self) -> u64 {
        self.net.inner.lock().watch_refusals
    }
}
