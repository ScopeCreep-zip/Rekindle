//! What the simulated network holds and what each node's app asked of
//! it, read by scenarios to assert on (plan E2.B): live route blobs,
//! replica values, and the per-node record and message counters.

use rekindle_records::port::RecordValue;

use crate::{records, routes, Netsim};

impl Netsim {
    /// The blobs of `node`'s live general routes, as a peer who reached
    /// it would hold them (for an outsider in a scenario).
    #[must_use]
    pub fn route_blobs(&self, node: usize) -> Vec<Vec<u8>> {
        let inner = self.net.inner.lock();
        let mut ids: Vec<&String> = inner
            .routes
            .iter()
            .filter(|(id, r)| r.node == node && id.contains("-g"))
            .map(|(id, _)| id)
            .collect();
        ids.sort();
        ids.into_iter().map(|id| routes::blob_of(id)).collect()
    }

    /// What a get from a node reaching every replica, with no local copy,
    /// answers at `key`/`subkey`: the newest value over the window,
    /// stopping at `get_value_count` matching replicas (`records` cites
    /// Veilid).
    #[must_use]
    pub fn network_get(&self, key: &str, subkey: u32) -> Option<RecordValue> {
        let inner = self.net.inner.lock();
        let record = inner.records.get(key)?;
        let all: Vec<usize> = (0..record.replicas.len().min(records::REPLICAS)).collect();
        records::get_from(record, &all, subkey)
    }

    /// How many inspects of `key` node `node`'s app has asked for, of any
    /// scope.
    #[must_use]
    pub fn app_inspects(&self, node: usize, key: &str) -> u64 {
        self.net.inner.lock().nodes[node]
            .app_inspects
            .get(key)
            .copied()
            .unwrap_or(0)
    }

    /// How many of those inspects were in a network scope (`SyncGet`,
    /// `SyncSet`, `UpdateGet`, `UpdateSet`), each a fanout; a `Local`
    /// inspect reads only the local store and sends nothing.
    #[must_use]
    pub fn app_network_inspects(&self, node: usize, key: &str) -> u64 {
        self.net.inner.lock().nodes[node]
            .app_network_inspects
            .get(key)
            .copied()
            .unwrap_or(0)
    }

    /// How many gets of `key` node `node`'s app sent to the network.
    #[must_use]
    pub fn app_gets(&self, node: usize, key: &str) -> u64 {
        self.net.inner.lock().nodes[node]
            .app_gets
            .get(key)
            .copied()
            .unwrap_or(0)
    }

    /// How many range-only pushes naming more than one subkey (a replica
    /// coalesced the changes) reached node `node`'s watch on `key`.
    #[must_use]
    pub fn coalesced_pushes(&self, node: usize, key: &str) -> u64 {
        let inner = self.net.inner.lock();
        inner.nodes[node]
            .outbound
            .coalesced
            .get(key)
            .copied()
            .unwrap_or(0)
    }

    /// The subkeys of `key` node `node`'s Veilid reported to its app and
    /// the app has not read from the network since (0.5.8 report marks,
    /// `outbound_watch_state.rs:51-55`): a later range-only report leaves
    /// them out.
    #[must_use]
    pub fn unread_marks(&self, node: usize, key: &str) -> Vec<u32> {
        let inner = self.net.inner.lock();
        inner.nodes[node]
            .outbound
            .states
            .get(key)
            .map_or_else(Vec::new, |s| s.unread.keys().copied().collect())
    }

    /// Node `node`'s fallback inspection interval for its watch on `key`
    /// (`fallback_inspect_interval`, 0.5.8 `outbound_watch_state.rs:413-418`):
    /// 30 s, doubling on each inspection that finds nothing up to 300 s;
    /// `None` before the watch is established.
    #[must_use]
    pub fn fallback_interval(&self, node: usize, key: &str) -> Option<std::time::Duration> {
        let inner = self.net.inner.lock();
        inner.nodes[node]
            .outbound
            .states
            .get(key)
            .map(|s| std::time::Duration::from_micros(s.interval()))
    }

    /// How many subkeys node `node`'s Veilid left out of a range-only
    /// report of `key` because the app had not read them from the network
    /// since they were last reported (0.5.8 report marks,
    /// `outbound_watch_state.rs:251-266`).
    #[must_use]
    pub fn suppressed_hints(&self, node: usize, key: &str) -> u64 {
        let inner = self.net.inner.lock();
        inner.nodes[node]
            .outbound
            .suppressed
            .get(key)
            .copied()
            .unwrap_or(0)
    }

    /// How many sets node `node`'s app has asked for, of any record.
    #[must_use]
    pub fn app_sets(&self, node: usize) -> u64 {
        self.net.inner.lock().nodes[node].app_sets
    }

    /// How many of node `node`'s sets asked for its local copy again at
    /// its seq (same data, same writer): re-sends, not new writes.
    #[must_use]
    pub fn app_resends(&self, node: usize) -> u64 {
        self.net.inner.lock().nodes[node].app_resends
    }

    /// How many of node `node`'s sets were new writes (a seq-advancing
    /// value): [`Self::app_sets`] less [`Self::app_resends`].
    #[must_use]
    pub fn app_new_sets(&self, node: usize) -> u64 {
        let inner = self.net.inner.lock();
        let slot = &inner.nodes[node];
        slot.app_sets - slot.app_resends
    }

    /// How many app calls node `node` made, of any kind.
    #[must_use]
    pub fn app_calls(&self, node: usize) -> u64 {
        self.net.inner.lock().nodes[node].app_calls
    }

    /// Each replica's value at `key`/`subkey`, in placement order.
    #[must_use]
    pub fn replica_values(&self, key: &str, subkey: u32) -> Vec<Option<RecordValue>> {
        let inner = self.net.inner.lock();
        inner.records.get(key).map_or_else(Vec::new, |r| {
            r.replicas
                .iter()
                .map(|x| x.values.get(subkey as usize).cloned().flatten())
                .collect()
        })
    }

    /// The single-subkey records the network holds (sealed pages, among
    /// others): `(key, value bytes)` as [`network_get`](Self::network_get)
    /// answers, sorted by key.
    #[must_use]
    pub fn single_subkey_values(&self) -> Vec<(String, Vec<u8>)> {
        let keys: Vec<String> = self
            .net
            .inner
            .lock()
            .records
            .iter()
            .filter(|(_, r)| r.replicas.first().is_some_and(|x| x.values.len() == 1))
            .map(|(k, _)| k.clone())
            .collect();
        let mut values: Vec<(String, Vec<u8>)> = keys
            .into_iter()
            .filter_map(|k| {
                let data = self.network_get(&k, 0)?.data;
                Some((k, data))
            })
            .collect();
        values.sort();
        values
    }
}
