//! A node's Veilid local store, copied and put back (plan E3.2 S7l): Veilid
//! keeps its record store in the node's table store, under the data root,
//! so a user who restores a backup of their profile directory also brings
//! back the record copies and seqs the node held then. The equivocation
//! scenario restores both together.

use std::collections::{HashMap, HashSet};

use rekindle_records::port::RecordValue;

use super::Netsim;

/// A copy of one node's local record store: its subkey values and the
/// records it created or opened.
#[derive(Debug, Clone)]
pub struct LocalStore {
    local: HashMap<(String, u32), RecordValue>,
    known: HashSet<String>,
}

impl Netsim {
    /// A copy of `node`'s local record store.
    #[must_use]
    pub fn local_store(&self, node: usize) -> LocalStore {
        let inner = self.net.inner.lock();
        let slot = &inner.nodes[node];
        LocalStore {
            local: slot.local.clone(),
            known: slot.known.clone(),
        }
    }

    /// Put `store` back as `node`'s local record store: every value the
    /// node stored since is gone from it (the network keeps its replicas).
    pub fn restore_local_store(&self, node: usize, store: LocalStore) {
        let mut inner = self.net.inner.lock();
        let slot = &mut inner.nodes[node];
        slot.local = store.local;
        slot.known = store.known;
    }
}

#[cfg(test)]
mod tests {
    use rekindle_records::port::RecordSchema;

    use crate::Netsim;

    /// A restored local store holds the value the node stored before the copy
    /// was taken, not the newer one; the network keeps the newer replica.
    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn a_restored_local_store_rolls_back_the_node_s_copies() {
        let (sim, nodes) = Netsim::nodes(1, 1);
        let pool = nodes[0].ports.records.start_session().unwrap();
        let (lease, key, _) = pool
            .create(&RecordSchema::Dflt { owner_subkeys: 1 }, None)
            .await
            .unwrap();
        pool.set(lease, 0, b"old".to_vec(), None).await.unwrap();
        let backup = sim.local_store(0);
        pool.set(lease, 0, b"new".to_vec(), None).await.unwrap();
        sim.restore_local_store(0, backup);
        let local = pool.get(lease, 0, false).await.unwrap().unwrap();
        assert_eq!(local.data.as_slice(), b"old");
        assert_eq!(sim.network_get(&key, 0).unwrap().data.as_slice(), b"new");
    }
}
