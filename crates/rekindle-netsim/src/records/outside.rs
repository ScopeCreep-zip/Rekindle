//! What happens to a record's replicas from outside the app (plan E2.B
//! slice S11, MED-3 and MED-4), after the pinned veilid-core
//! (`~/veilid/veilid-core/src`):
//!
//! - **churn:** a record's replicas are the [`super::replicas::REPLICAS`]
//!   nodes closest to its key that a fanout counts (`consensus_width`,
//!   `rpc_processor/fanout/fanout_call.rs:292-330`); nodes joining closer to
//!   the key push the farthest out. [`crate::Fault::ReplicaChurn`] puts
//!   `count` empty stores closest (placement order is closeness) and drops
//!   the `count` farthest, their values and watches with them; each
//!   newcomer sits on the side of the replica it displaced. A watcher's
//!   check tick places its watch on the newcomers and inspects them
//!   ([`super::watches`], [`super::catchup`]); rehydration and the pool's
//!   writes refill them;
//! - **equivocation:** [`crate::Netsim::forge_write_to`] is a writer the
//!   schema admits sending one value at one seq to some replicas and
//!   another to others; each replica applies its own rule: the size rule
//!   ([`super::size`]) and the first value at a seq wins
//!   (`storage_manager/set_value.rs:700-719`).

use rekindle_records::port::{PortError, RecordValue};

use super::{admits, public_text, size, stores, watches, writer_public};
use crate::Netsim;

impl Netsim {
    /// `count` of `key`'s replicas churn (see the module notes).
    pub(crate) fn churn(&self, key: &str, count: usize) {
        let mut guard = self.net.inner.lock();
        let inner = &mut *guard;
        let Some(record) = inner.records.get_mut(key) else {
            return;
        };
        let count = count.min(record.replicas.len());
        let subkeys = record.replicas.first().map_or(0, |r| r.values.len());
        let gone: Vec<_> = record
            .replicas
            .drain(record.replicas.len() - count..)
            .collect();
        for (i, old) in gone.into_iter().enumerate() {
            inner.lru += 1;
            let fresh = super::Replica {
                home: old.home,
                store: usize::MAX / 2 + usize::try_from(inner.lru).unwrap_or(0),
                used: 0,
                values: vec![None; subkeys],
                watches: Vec::new(),
            };
            record.replicas.insert(i, fresh);
        }
    }

    /// A writer the schema admits stores `data` at `seq` on the `replicas`
    /// of `key` (placement indices), each by Veilid's replica rule; how many
    /// stored it.
    ///
    /// # Errors
    /// The record is unknown, or the schema does not admit the writer.
    pub fn forge_write_to(
        &self,
        key: &str,
        subkey: u32,
        data: Vec<u8>,
        (writer, seq): (&str, u32),
        replicas: &[usize],
    ) -> Result<usize, PortError> {
        let public = writer_public(writer)?;
        let mut guard = self.net.inner.lock();
        let inner = &mut *guard;
        let record = inner
            .records
            .get_mut(key)
            .ok_or_else(|| PortError::Unreachable(key.into()))?;
        if !admits(record, subkey, &public) {
            return Err(PortError::NotWritable(key.into()));
        }
        let value = RecordValue {
            seq,
            data,
            writer: public_text(&public),
        };
        let at = subkey as usize;
        let mut stored = Vec::new();
        for r in replicas {
            let Some(replica) = record.replicas.get_mut(*r) else {
                continue;
            };
            if !size::admits(&record.schema, &replica.values, at, &value) {
                continue;
            }
            if let Some(slot) = replica.values.get_mut(at) {
                if slot.as_ref().is_none_or(|v| v.seq < seq) {
                    *slot = Some(value.clone());
                    stored.push(*r);
                }
            }
        }
        watches::mark(record, &stored, subkey, usize::MAX);
        stores::admit(inner, key, &stored);
        Ok(stored.len())
    }

    /// How many inbound RPCs `node`'s full queue dropped (Veilid's
    /// `rpc.queue_size`, [`crate::network`]).
    #[must_use]
    pub fn rpc_dropped(&self, node: usize) -> u64 {
        self.net
            .inner
            .lock()
            .rpc_dropped
            .get(&node)
            .copied()
            .unwrap_or(0)
    }
}
