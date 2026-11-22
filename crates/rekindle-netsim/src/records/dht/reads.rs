//! The span of a read on [`SimDht`]: what it costs depends on whether a
//! reached replica holds the value.

use super::super::{reachable, replicas};
use super::SimDht;
use crate::latency::{self, Span};

impl SimDht {
    /// Whether a replica the node reaches holds a value at any of
    /// `subkeys` (all when `None`).
    pub(super) fn held(&self, key: &str, subkeys: Option<&[u32]>) -> bool {
        let inner = self.net.inner.lock();
        inner.records.get(key).is_some_and(|r| {
            let reach = reachable(&r.replicas, &inner.faults, self.node);
            replicas::holds(&r.replicas, &reach, subkeys)
        })
    }

    /// A read's latency: [`latency::GET`] when a reached replica holds a
    /// value at any of `subkeys` (all when `None`), else the fanout's
    /// [`latency::DEADLINE`].
    pub(super) fn read_span(&self, key: &str, subkeys: Option<&[u32]>) -> Span {
        if self.held(key, subkeys) {
            latency::GET
        } else {
            latency::DEADLINE
        }
    }
}
