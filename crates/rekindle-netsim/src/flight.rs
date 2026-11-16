//! An own-route allocation in flight on the simulated network, traced at
//! its start, its end, and as cut when dropped before it ([`crate::routes`]).

use rekindle_records::port::RouteClass;

use crate::network::Network;
use crate::routes::{allocation_entry, ALLOCATION_CUT, ALLOCATION_ENDS, ALLOCATION_STARTS};

/// An allocation in flight: traced at its start and end, and as cut when
/// dropped before its end.
pub(crate) struct InFlight<'a> {
    node: usize,
    class: RouteClass,
    net: &'a Network,
    ended: bool,
}

impl<'a> InFlight<'a> {
    pub(crate) fn start(node: usize, class: RouteClass, net: &'a Network) -> Self {
        let flight = Self {
            node,
            class,
            net,
            ended: false,
        };
        flight.push(ALLOCATION_STARTS);
        flight
    }

    pub(crate) fn end(&mut self) {
        self.ended = true;
        self.push(ALLOCATION_ENDS);
    }

    /// Trace `event`. Allocations that start (or are cut) together run in
    /// the order tokio wakes them, which the seed does not fix, so a run of
    /// these entries at one instant is kept sorted.
    fn push(&self, event: &str) {
        let entry = (self.node, allocation_entry(event, self.class));
        let now = tokio::time::Instant::now();
        let mut guard = self.net.inner.lock();
        let inner = &mut *guard;
        let len = inner.trace.len();
        let start = match inner.alloc_tail {
            Some((at, start, end)) if at == now && end == len => start,
            _ => len,
        };
        let place = start
            + inner.trace.entries()[start..]
                .iter()
                .filter(|e| **e <= entry)
                .count();
        inner.trace.insert(place, entry.0, entry.1);
        inner.alloc_tail = Some((now, start, len + 1));
    }
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        if !self.ended {
            self.push(ALLOCATION_CUT);
        }
    }
}
