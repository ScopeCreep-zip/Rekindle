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

    /// Watches a replica refused at its watch limit, since the start.
    #[must_use]
    pub fn watch_refusals(&self) -> u64 {
        self.net.inner.lock().watch_refusals
    }
}
