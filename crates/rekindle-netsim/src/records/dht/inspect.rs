//! Local and network revision reports for a simulated record.

use rekindle_records::port::{DhtCallError, DhtErrorKind, RecordReport, ReportScope};

use super::{error, no_answer, reachable, replicas, stores, subkey_count, SimDht};

impl SimDht {
    pub(super) async fn inspect_record(
        &self,
        key: &str,
        subkeys: Option<&[u32]>,
        scope: ReportScope,
    ) -> Result<RecordReport, DhtCallError> {
        self.net.count_app_inspect(self.node, key);
        if scope != ReportScope::Local {
            self.net.count_app_network_inspect(self.node, key);
        }
        self.opened(key)?;
        let _slot = if scope == ReportScope::Local {
            None
        } else {
            Some(
                self.network(|| self.read_span(key, subkeys), key, &[])
                    .await?,
            )
        };
        if scope != ReportScope::Local && self.reach(key).is_empty() {
            return Err(no_answer("inspect"));
        }
        let mut inner = self.net.inner.lock();
        if scope != ReportScope::Local {
            let reach = inner.records.get(key).map_or_else(Vec::new, |r| {
                reachable(&r.replicas, &inner.faults, self.node)
            });
            stores::touch(&mut inner, key, &reach);
        }
        let record = inner
            .records
            .get(key)
            .ok_or_else(|| error(DhtErrorKind::KeyNotFound, key.to_string()))?;
        let subkeys: Vec<u32> = match subkeys {
            // Truncated to the schema, as a sorted range set
            // (`inspect_record.rs:20`).
            Some(s) => {
                let all = subkey_count(&record.schema);
                let mut s: Vec<u32> = s.iter().copied().filter(|s| *s < all).collect();
                s.sort_unstable();
                s.dedup();
                s
            }
            None => (0..subkey_count(&record.schema)).collect(),
        };
        let slot = &inner.nodes[self.node];
        // UpdateSet simulates a set: each local seq + 1
        // (0.5.8 `inspect_record.rs:316-321`).
        let bump = u32::from(scope == ReportScope::UpdateSet);
        let local_seqs: Vec<Option<u32>> = subkeys
            .iter()
            .map(|s| {
                slot.local
                    .get(&(key.to_string(), *s))
                    .map(|v| v.seq.saturating_add(bump))
            })
            .collect();
        let reach = if scope == ReportScope::Local {
            Vec::new()
        } else {
            reachable(&record.replicas, &inner.faults, self.node)
        };
        // The fanout starts from the local result for the Update scopes,
        // from nothing for the Sync ones (`:328-332`); Local reports none.
        let from_local = matches!(scope, ReportScope::UpdateGet | ReportScope::UpdateSet);
        // The set scopes count `set_value_count` holders, the get scopes
        // `get_value_count` (0.5.8 `inspect_record.rs:426-450`).
        let count = if matches!(scope, ReportScope::UpdateSet | ReportScope::SyncSet) {
            replicas::SET_COUNT
        } else {
            replicas::GET_COUNT
        };
        let network_seqs = subkeys
            .iter()
            .zip(&local_seqs)
            .map(|(s, l)| match scope {
                ReportScope::Local => None,
                _ => {
                    replicas::consensus(
                        &record.replicas,
                        &reach,
                        *s as usize,
                        l.filter(|_| from_local),
                        count,
                    )
                    .0
                }
            })
            .collect();
        // Its own offline queue, queued or in flight, and the sets in
        // flight (0.5.8 `inspect_record.rs:284-290`).
        let offline_subkeys = subkeys
            .iter()
            .copied()
            .filter(|s| {
                let at = (key.to_string(), *s);
                slot.pushes.contains_key(&at) || slot.setting.contains_key(&at)
            })
            .collect();
        Ok(RecordReport {
            subkeys,
            local_seqs,
            network_seqs,
            offline_subkeys,
        })
    }
}
